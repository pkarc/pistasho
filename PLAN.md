# pistasho: plan de implementación

Cada tarea está pensada para una sesión de Claude Code: un objetivo, qué tocar y cómo saber que está lista. Hazlas en orden dentro de cada fase, salvo que se indique lo contrario. Marca `[x]` al terminar y anota en **Notas** lo que cambió respecto al plan.

Tamaños: **S** = menos de una sesión, **M** = una sesión, **L** = dos o más sesiones (conviene partirla).

Reglas para todas las tareas:
- `cargo fmt --check`, `cargo clippy --all-targets` sin warnings nuevos y `cargo test` pasan.
- Toda lógica pura nueva lleva pruebas unitarias.
- Nada ejecuta el comando sugerido (ver principios en `CLAUDE.md`).
- Idioma: código y comentarios en inglés; toda cadena visible de la herramienta lleva inglés y español (`tr("English", "Español")`); el README se actualiza en sus dos versiones. Este plan, las decisiones y `CLAUDE.md` van solo en español.

---

## Fase 1 · Cierre del buscador

### [x] 1.1 Compilar y corregir · S
El código nunca se compiló con sus dependencias.
- `cargo build`, corregir errores de API (ratatui 0.29, nucleo-matcher 0.3, ureq 2, zip 2, clap 4).
- Si una versión fijada da problemas, actualizarla y anotarlo.

**Lista cuando:** compila, clippy y tests pasan, y CI queda en verde en GitHub.

### [ ] 1.2 Prueba de punta a punta · S
- `pistasho config` → `pistasho update` → `pistasho query tar` devuelve ejemplos de tar en español.
- Buscador: `pistasho search` directo (elegir, rellenar marcadores, el comando queda en el prompt y el cursor al final) y con el popup de tmux. **No** pulir el widget de `Ctrl-G`: se va a quitar (ADR 0001).
- Popup de tmux: el comando llega al panel correcto sin Enter, también dentro de una sesión ssh.
- Cancelar con Esc deja el prompt como estaba.

**Lista cuando:** todo lo anterior funciona en la máquina del usuario y los problemas encontrados están corregidos.

### [x] 1.3 Rendimiento base · S
- Medir el tiempo desde `Ctrl-G` hasta ver resultados (cargar `index.json` + historial) y el tiempo por tecla con ~30k entradas.
- Si la carga pasa de ~150 ms: cambiar el índice a un formato binario (bincode / postcard) o adelantar la parte de SQLite de la tarea 2.1.

**Lista cuando:** hay números anotados en `docs/eval.md`, la apertura queda en menos de ~150 ms y cada tecla en menos de ~30 ms.

### [x] 1.4 Línea base de `eval` · S
- Correr `pistasho eval` y guardar top-1 / top-3 en `docs/eval.md`.
- Ampliar `eval/questions.toml` hacia ~100 preguntas con comandos que el usuario realmente usa (puede ser una tarea continua).

**Lista cuando:** `docs/eval.md` tiene la línea base de la fase 1 con fecha.

---

## Fase 2 · Modo pregunta sin IA

### [x] 2.1 Índice en SQLite + FTS5 · M
- Agregar `rusqlite` con `bundled`. Base en `~/.local/share/pistasho/pistasho.db`.
- Tablas: `entries` (cmd, desc, alt, tool, source, origin, count), tabla virtual `entries_fts` (FTS5 sobre desc, alt, tool, tags) y `meta` (versión del esquema e idiomas).
- `update` escribe en SQLite. Cheats e historial se sincronizan al abrir (comparando la fecha de modificación de los archivos).
- El buscador difuso de la fase 1 sigue igual, pero leyendo de SQLite.
- Normalizar acentos en FTS5 (`tokenize = "unicode61 remove_diacritics 2"`).

**Lista cuando:** `index.json` desaparece, el buscador funciona igual o más rápido, y una prueba confirma que "compresión" encuentra "comprimir" o "compression" vía `alt`.

### [x] 2.2 `pistasho ask` con BM25 · M
- Subcomando `ask "<pregunta>"` que devuelve el mejor candidato por BM25 (`bm25(entries_fts)`), con `--json` para depurar (candidatos y puntuaciones).
- `ask` debe poder devolver también los N mejores candidatos con su origen (los necesita la lista de alternativas), no solo el mejor.
- Quitar palabras vacías en español e inglés de la pregunta antes de buscar ("qué", "cómo", "de", "the"…).
- Subir el peso de las entradas `mío` y de las del historial que se usan seguido.
- `eval` acepta `--mode fuzzy|bm25` para comparar.

**Lista cuando:** `pistasho eval --mode bm25` supera claramente a la línea base de la 1.4 y el resultado está anotado en `docs/eval.md`.

### [x] 2.3a Respuesta directa · M
- `init zsh` agrega el widget `_pistasho_ask` en `keys.ask` (`Ctrl-Space`) y **quita** el widget de `Ctrl-G` (y con él `keys.search`).
- El prompt cambia a ` ✦ pregunta ` con una pista a la derecha (`⏎ preguntar · tab explorar · esc salir`). Enter muestra el spinner `resolviendo…`, resuelve y deja el mejor comando en `BUFFER` con el prompt normal; Esc vuelve al prompt original sin cambios.
- Opción recomendada: el binario lee la pregunta en `/dev/tty` dentro de la línea (sin pantalla alternativa) y devuelve el comando por stdout. Alternativa: `zle recursive-edit` con un `PS1` temporal.
- Debajo del prompt, mensaje con `zle -M`: `⚡ local · tldr/fc · Ctrl-Space: ver 5 más`. El comando puede ser multilínea (historial), así que conviene separar los metadatos con NUL (`--format zsh`, `parts=("${(@0)out}")`) o escribirlos en otro descriptor; que lo decida la implementación.
- Si el comando trae marcadores, se piden igual que en el buscador actual.

**Lista cuando:** el paso 1 del GIF (`docs/pistasho-modo-pregunta.gif`) funciona en zsh, con y sin texto previo en el prompt.

### [ ] 2.3b Alternativas · M
- Estado del widget en variables de shell `_PISTASHO_LAST_Q` / `_PISTASHO_LAST_CMD`, que se limpian al terminar la línea (`add-zle-hook-widget zle-line-finish`) para que no sobrevivan a un Enter. Al presionar `Ctrl-Space`: si `$BUFFER == $_PISTASHO_LAST_CMD` y no está vacío, se abren las alternativas; si no, pregunta nueva.
- `ask --alternatives --current "<cmd>" "<pregunta>"` abre la lista e imprime el comando elegido (código 130 si se cancela).
- Refactor de `tui.rs` de buscador a **lista de candidatos** con dos modos (`Alternatives { question, current }`, `Explore`) más el relleno de marcadores que ya existe, **en línea** debajo del prompt (`Viewport::Inline`, ya hecho en 5.1). Título `otras opciones · «<pregunta>»`, campo de filtro `❯` con filtro difuso en vivo, filas con etiqueta + comando (marcadores resaltados), el comando actual incluido. `↑↓` mover, `Enter` reemplaza (pidiendo marcadores), `Esc` deja el actual. Al reemplazar, debajo del prompt: `✓ reemplazado · <origen>`.

**Lista cuando:** los pasos 2 y 3 del GIF funcionan, y editar el `BUFFER` o ejecutar el comando hace que el siguiente `Ctrl-Space` sea una pregunta nueva.

### [ ] 2.3c Explorar · S
- `Tab` con el campo vacío en el modo pregunta → `ask --explore`: sin pregunta, ordenada como `empty_query_order` (mío → historial reciente → tldr), con filtro difuso al escribir. `Enter` elige (con relleno de marcadores), `Tab` inserta la plantilla sin rellenar, `Esc` sale.

**Lista cuando:** el paso 4 del GIF funciona.

### [ ] 2.3d Popup de tmux en modo pregunta · S
- `init tmux` abre el modo pregunta en el popup, con `Tab` para explorar igual que en zsh. Tras resolver muestra el resultado con `⏎ enviar · ↑↓ ver más` (no hay `BUFFER` previo, así que no hay paso de alternativas). El resultado se envía con `tmux send-keys -l`.

**Lista cuando:** el comando llega al panel sin Enter, también dentro de una sesión ssh.

### [ ] 2.4 Verificación de binarios · S
- Módulo `system::which(bin)` que busca en `PATH`, sin lanzar procesos.
- Extraer los binarios de un comando: primera palabra, palabras después de `|`, `&&`, `;`, `$(`, `xargs`, y saltar `sudo` / `env` / asignaciones `VAR=x`.
- `ask` marca cada candidato como disponible o faltante.

**Lista cuando:** hay pruebas para `sudo tar …`, `ss -tlnp | grep :80`, `find . | xargs rg x` y `FOO=1 make`.

### [ ] 2.5 Detección de OS y paquetes · M
- Leer `/etc/os-release` (`ID`, `ID_LIKE`, `PRETTY_NAME`) → gestor: apt, pacman, dnf o zypper.
- Tabla curada `data/packages.toml` de comando → paquete por familia, incluidos los casos raros: `rg`→`ripgrep`; `fd`→`fd-find` en Debian, donde el binario es `fdfind`; `bat`→`bat`, con binario `batcat` en Debian; `ncdu`, `tree`, `jq`, `htop`, `btop`, `delta`→`git-delta`…
- Verificar que el paquete exista antes de proponerlo: `apt-cache policy X`, `pacman -Si X`, `dnf info X`, `zypper info X`.
- Si el binario instalado tiene otro nombre (`fdfind`), reescribir el comando o avisar.

**Lista cuando:** probado en Ubuntu y Arch (contenedores sirven), con pruebas de la tabla y del parser de os-release.

### [ ] 2.6 Flujo de instalación · M
- Caja de confirmación del GIF 2: comando, sistema, `Instalar: sudo apt install -y X`, opciones `[s]` instalar, `[a]` alternativa, `[esc]` cancelar.
- Al confirmar: correr la instalación en primer plano heredando la tty (sudo pide la contraseña y la salida de apt se ve). Al terminar, verificar el binario otra vez, mostrar `✓ X instalado` y completar el comando.
- Si falla: mostrar el error y no tocar el prompt.
- Nunca instalar sin confirmación, ni con `--yes` en una configuración por defecto.

**Lista cuando:** el caso `ncdu` del GIF 2 funciona de punta a punta en una máquina donde falta ncdu.

### [ ] 2.7 Alternativas sin instalar · S
- Tabla curada de alternativas con herramientas base: `tree`→`find {{dir}} -maxdepth 2 | sort`, `ncdu`→`du -h -d1 {{dir}} | sort -h`, `rg`→`grep -rn`, `fd`→`find -name`, `jq .`→`python3 -m json.tool`, `htop`→`top`…
- La opción `[a]` usa la tabla y verifica que los binarios de la alternativa existan.

**Lista cuando:** el caso `tree` del GIF 3 funciona.

### [ ] 2.8 Reglas de riesgo · M
- Módulo `risk` con patrones: `rm -rf` (sobre todo `/`, `~` o `*`), `dd … of=/dev/…`, `mkfs*`, `wipefs`, `shred`, `chmod/chown -R` sobre `/`, `> /dev/sd*`, fork bomb, `:(){`, `git push --force` a main, `DROP TABLE`…
- Niveles: `ok`, `cuidado`, `destructivo`.
- Si un marcador es un dispositivo (`{{dispositivo}}`, `/dev/sdX`): ofrecer solo los dispositivos extraíbles de `lsblk -J -o NAME,MODEL,SIZE,FSTYPE,MOUNTPOINT,RM`.
- En `destructivo`, caja roja del GIF 3. El comando queda en el prompt con `⚠ destructivo` en `RPROMPT` y resaltado (`region_highlight`).
- Se aplica a todo comando que salga de `ask`, y también al buscador.

**Lista cuando:** hay pruebas por patrón, incluidos falsos positivos obvios (`rm -rf ./build` es `cuidado`, no `destructivo`), y el caso de la USB del GIF 3 funciona.

### [ ] 2.9 Embeddings multilingües · L (partir en 2.9a / 2.9b)
- **2.9a:** elegir el modelo (familia `multilingual-e5-small`, cuantizado) y la vía de ejecución (`fastembed` o `ort` directo). Medir RAM, tamaño y latencia por pregunta en la máquina del usuario. Descargarlo en `update` y no incluirlo en el binario.
- **2.9b:** precalcular los vectores de todas las entradas en `update` y guardarlos en SQLite como blob. Al preguntar, calcular el vector de la pregunta, similitud coseno (con fuerza bruta sobre unos 30k vectores basta), y fusionar con BM25 por rangos (RRF, k=60).
- Opción `search.embeddings = true|false` en la configuración y en el asistente.

**Lista cuando:** `eval --mode hybrid` supera a `bm25`, la latencia de `ask` sigue en menos de ~150 ms, y está anotado en `docs/eval.md`. Si no mejora, dejarlo apagado por defecto y anotar por qué.

### [ ] 2.10 Cierre de fase 2 · S
- `eval` final de la fase, README actualizado y un GIF real con `vhs` del modo pregunta que cubra los 4 pasos del ADR 0001 (respuesta, alternativas, filtrar + reemplazar, explorar).

---

## Fase 3 · Modelo de decisión (opcional)

> Solo se empieza si en `docs/eval.md` el top-1 híbrido está por debajo de ~85% o si decidir "ninguno sirve" con umbrales de BM25 resulta poco confiable.

### [ ] 3.1 Interfaz `Decide` + Jev / laya-serve por HTTP · M
- Trait `Decide` con `choice(estado, opciones) -> (índice, prob)` y `noul(estado, pregunta) -> prob`.
- Cliente HTTP para la API compatible con Jev, que sirve para Jev y para laya-serve; `decide.endpoint`, `decide.api_key_env`.
- `ask` usa choice entre los 10 mejores candidatos + "ninguno".
- `eval --mode hybrid+decide`.

**Lista cuando:** funciona contra laya-serve (por ejemplo en el Proxmox del usuario) y está medido en `eval`.

### [ ] 3.2 Datos de entrenamiento · M
- Ampliar el conjunto de prueba con pares sintéticos (pregunta, candidatos, correcto) generados desde tldr y revisados. Separar entrenamiento y prueba.

### [ ] 3.3 Fine-tuning y calibración de Laya · L
- Ajustar el checkpoint multilingüe y calibrar la temperatura. Exportar a ONNX int8.

**Lista cuando:** supera al híbrido sin Laya por 5–10 puntos en el conjunto de prueba.

### [ ] 3.4 Laya local con daemon · L
- `pistasho daemon`: socket unix en `$XDG_RUNTIME_DIR/pistasho.sock`, carga el modelo una vez, se levanta solo en la primera llamada y se apaga tras un tiempo sin uso.
- `ort` en Rust, sin Python.

**Lista cuando:** la primera llamada en frío tarda menos de 2 s y las siguientes menos de ~300 ms en CPU. Si no se logra, se queda solo la vía laya-serve.

### [ ] 3.5 Riesgo y alternativas con noul · S
- Combinar las reglas fijas de 2.8 con `noul("¿es destructivo?")`. Las reglas siempre ganan; el modelo solo agrega avisos.
- `noul("¿la alternativa cumple la petición?")` para la opción `[a]`.

### [ ] 3.6 Decisión · S
- Con los números en mano, decidir si Laya queda activado por defecto, opcional o se descarta. Anotarlo en `docs/eval.md`.

---

## Fase 4 · LLM de respaldo

### [ ] 4.1 Interfaz `Generate` + Anthropic · M
- Llamada con salida estructurada: `{command, binaries[], package?, risk, explanation}`.
- Prompt de sistema con el OS, el gestor de paquetes, el shell y "comando de una línea, sin ejecutar nada". **Nunca** incluir historial, rutas ni archivos.
- Key desde la variable de `llm.api_key_env`; timeout corto y mensaje claro si falla.

**Lista cuando:** el caso `.jpeg → .jpg` del GIF 3 funciona.

### [ ] 4.2 Ollama · S
- Misma interfaz contra `llm.endpoint` / `llm.model`, con formato JSON.

### [ ] 4.3 Cuándo llamar al LLM · S
- Llamarlo solo si la vía local no encontró nada confiable: BM25 / híbrido bajo un umbral, o "ninguno" del modelo de decisión.
- Mostrar `✦ llm · modelo · 1.1 s` debajo del prompt.

### [ ] 4.4 Validación y caché · S
- Todo comando del LLM pasa por 2.4 (binarios), 2.5/2.6 (instalación) y 2.8 (riesgo) antes de llegar al prompt.
- Caché pregunta normalizada → comando en SQLite. Al repetir la pregunta se resuelve como `⚡ local · caché`.
- Opción para guardar en `cheats.toml` un comando del LLM que te sirvió.

**Lista cuando:** la segunda vez que se hace la misma pregunta no se llama al LLM.

---

## Fase 5 · Pulido y lanzamiento

### [x] 5.1 Buscador compacto · M (absorbida por 2.3b)
- Absorbida por 2.3b (ADR 0001): la lista en línea es el mismo componente. El viewport en línea se implementó aquí, antes de tiempo, y 2.3b lo reutiliza.
- `Viewport::Inline` de ratatui debajo del prompt, como `fzf --height 40%`. Opción `ui.height` y la pantalla completa como alternativa.

**Lista cuando:** en zsh el buscador abre debajo del prompt, y elegir o cancelar deja el prompt y el cursor intactos, también con el prompt en la última fila. (Hecha antes de tiempo, antes de la fase 2.)

### [ ] 5.2 Empaquetado · M
- cargo-dist para releases de GitHub (binarios Linux x86_64 / aarch64), paquete AUR y `cargo install pistasho` (publicar el crate).

### [ ] 5.3 Demos reales · S
- Cambiar los GIFs simulados por grabaciones de `vhs` (archivos `.tape` en `docs/`).

### [ ] 5.4 Documentación y lanzamiento · S
- README completo, sección de privacidad, cómo contribuir con cheats y preguntas a `eval`, y un CHANGELOG.

---

## Notas

<!-- Anota aquí decisiones o cambios de plan, con fecha. -->

- 2026-11-19 · 1.1: compiló a la primera con las versiones fijadas, sin errores de API ni cambios de versión. Único ajuste: warning `unnecessary_sort_by` de clippy en `history.rs`. Se versionó `Cargo.lock` (es un binario). CI en verde. `cargo install` deja el binario en `~/.cargo/bin`, que no estaba en el `PATH`.
- 2026-11-19 · Idioma: el proyecto ahora va en inglés por defecto (README, CLAUDE.md, PLAN.md). Las versiones en español se conservan como `README.es.md`, `CLAUDE.es.md` y `PLAN.es.md` y deben mantenerse alineadas. Las cadenas de la interfaz y los comentarios del código se migraron al inglés en el mismo cambio. La interfaz en español sigue disponible con `language = "es"` / `PISTASHO_LANG=es` (`src/i18n.rs`); toda cadena nueva de cara al usuario necesita ambos idiomas. El archivo de eval ahora es `eval/questions.toml` con las claves `question` / `tools` (las preguntas se quedan en español a propósito). Los `languages` por defecto ahora son `["en", "es"]`.
- 2026-11-19 · 1.2 (parcial): verificado sin interfaz en servidores tmux aislados: `config` → `update` (6852 páginas, 34878 ejemplos) → `query`, el buscador + relleno + envío a un panel sin Enter, y el widget de zsh (`Ctrl-G` con texto previo, elegir, cursor al final, Esc deja el prompt intacto). Arreglo: el popup de tmux corre con el `PATH` del servidor de tmux, que no tenía `~/.cargo/bin`, así que `init tmux` ahora incrusta la ruta absoluta del binario. Falta verificar a mano: el `display-popup` real y dentro de una sesión ssh. Hallazgo para 2.1/2.2: el mismo comando de tldr aparece dos veces (en y es) cuando se traducen los nombres de los marcadores (`{{path/to/file}}` vs `{{ruta/al/archivo}}`), porque la clave de unión es el comando normalizado. Con `languages = ["en", "es"]` las descripciones de `query tar` salen en inglés (el criterio de la 1.2 decía español; usa `["es", "en"]` para eso).
- 2026-11-19 · 5.1 adelantada: el widget de zsh ahora usa `Viewport::Inline` (`ui.height`, por defecto `40%`, o `--height`; se pregunta en `pistasho config`). El popup de tmux (`--send-to`) mantiene la pantalla completa. ratatui/crossterm leen la posición del cursor por stdout, que en el widget es el pipe de `$(...)` (el primer intento falló tras 2 s de espera), así que `tui::run` redirige stdout al tty mientras corre. Verificado en servidores tmux aislados con el prompt arriba y en la última fila: elegir, relleno, cancelar, cursor al final del buffer. Además: la descripción de `Cargo.toml` ahora está en inglés y `libc` es dependencia directa.
- 2026-11-19 · ADR 0001 aplicado: Ctrl-Space única entrada; buscador → lista de alternativas. La tarea 2.3 se divide en 2.3a–d; 5.1 absorbida por 2.3b (el viewport en línea ya existe); 2.2 debe devolver N candidatos; 1.2 ya no pule el widget de `Ctrl-G`. Ver `docs/adr/0001-question-mode-single-entry.md`. El README sigue describiendo `Ctrl-G` porque es lo que funciona hoy; actualizarlo en 2.3a.
- 2026-11-19 · Idioma, revisado: se corrige la nota anterior. Código, comentarios y README (en dos versiones) en inglés; la herramienta en inglés por defecto con español elegible (`language = "es"`, `PISTASHO_LANG`) o detectado (`language = "auto"`, desde `LC_ALL` / `LC_MESSAGES` / `LANG`). Las decisiones, el plan y el contexto del agente se mantienen solo en español: `CLAUDE.md`, `PLAN.md` y `docs/adr/` son un único archivo en español y desaparecen los gemelos en inglés (`CLAUDE.es.md`, `PLAN.es.md` y el ADR en inglés ya no existen).
- 2026-11-19 · 1.3 y 1.4 hechas (números en `docs/eval.md`). Apertura 67 ms (meta < 150), así que el índice se queda en JSON. Cada tecla estaba en ~175 ms (meta < 30): ahora ~20–28 ms de media (máx. 46) con búsqueda incremental por prefijos, hilos y `alt` recortado a 150 caracteres (`ALT_SEARCH_CHARS`), medido con `eval` para no perder precisión. Línea base de la fase 1: top-1 18%, top-3 28%. Se añadió `query --timing`. Pendiente de 1.4 como tarea continua: ampliar `eval/questions.toml` hacia ~100 preguntas. Ojo: los hilos dependen de los núcleos (medido en 16).
- 2026-11-19 · 2.1 hecha. `rusqlite` 0.40 con `bundled`; `src/db.rs` con `entries`, `entries_fts` (FTS5 externo con triggers, `unicode61 remove_diacritics 2`) y `meta`. Desvíos del plan: las columnas se llaman `description` y `uses` (palabras reservadas); el FTS cubre `description`, `alt` y `tool` (las etiquetas de los cheats ya van en `alt`, no hay columna `tags`); `fts_query` aplica un stemming ligero (las palabras de más de 4 letras pierden las últimas 3, mínimo 4) porque con prefijo exacto `comprimir` no llegaba a `comprime` ni `compresión` a `compression`; esa heurística se afina en 2.2 con `eval`. El criterio "compresión encuentra comprimir o compression" se cumple con tests (`compresión` → `compression` en la descripción; `comprimir` → `comprime` / `comprimido` en `alt`; acentos ignorados). `index.json` desaparece (`update` lo borra). Carga ~45 ms desde SQLite contra ~50 ms con JSON; apertura 62–66 ms; `eval` idéntico (18% / 28%). Cheats e historial se resincronizan por huella de archivo; probado en un entorno aislado (XDG_*) con alta, cambio y vaciado. Hallazgo y arreglo: el zsh del usuario guarda el historial en `~/.histfile` con `HISTFILE` sin exportar, así que pistasho nunca lo veía (0 entradas `hist`); ahora el widget exporta `HISTFILE` al comando y `history_path` prueba las rutas habituales (2216 entradas cargadas). Pendiente para 2.2: el ranking BM25 sobre la base real todavía es ruidoso (`puertos escuchando` trae `rcat`, `mysqld`): hacen falta palabras vacías y pesos por columna.
- 2026-11-19 · 1.2, bug del popup de tmux: el usuario vio que al elegir un comando no llegaba al prompt. Causa: `display-popup` no expande `#{pane_id}` en su comando, así que pistasho recibía el texto literal como destino de `tmux send-keys` y fallaba en silencio (`-E` cierra el popup y esconde el error). Mis pruebas anteriores pasaban el id a mano y por eso no lo vieron. Arreglo: `init tmux` genera `bind-key g run-shell -b "tmux display-popup -c #{client_name} -t #{pane_id} … --send-to #{pane_id}"`, porque `run-shell` sí expande los formatos. Verificado con un cliente tmux real en un pty: elegir, rellenar marcadores, `Tab` y cancelar con Esc. Hay un test que fija la forma del binding. El usuario debe sustituir la línea vieja de su `~/.tmux.conf`. Pendiente solo: confirmarlo a mano dentro de una sesión ssh.
- 2026-11-19 · 2.2 hecha. `pistasho ask "<pregunta>"` imprime el mejor comando (código 1 si no hay coincidencia); `--json -n N` imprime los N candidatos con origen y puntuación (los que necesitará 2.3b). `eval --mode fuzzy|bm25` (por defecto bm25) y `--weights` para afinar. Resultado: sobre las 40 originales BM25 saca 28% / 45% contra la línea base 18% / 28%; sobre 101 preguntas 37% / 51% contra 28% / 41% (todo en `docs/eval.md`). Cambios respecto al plan: (1) el esquema de la base pasa a la versión 2 con `cmd_words` (palabras del comando sin marcadores), y el usuario debe correr `pistasho update` una vez; (2) se amplió `eval/questions.toml` a 101 preguntas porque con 40 las comparaciones eran ruido; (3) las traducciones de tldr ahora se unen por la forma del comando sin nombres de marcadores, que era el defecto anotado en 1.2 (−4000 duplicados); (4) el historial no indexa su descripción generada ("usado N veces"). Las palabras vacías son una lista fija en `src/ask.rs`. Lo que falla es sobre todo la distancia semántica español → inglés (`montar una partición`, `formatear una usb`), que es trabajo de 2.9. Los "boost" de `mío` (×1.3) y de historial frecuente (+3% por uso, hasta 10) no se pueden medir con `eval` (no hay cheats ni historial reproducibles) y quedan como suposiciones razonables con test.
- 2026-11-20 · 2.2, bug visto por el usuario ("no hace nada"): `pistasho ask` devolvía su propia invocación guardada en el historial (`pistasho ask comprimir una carpeta en zip` contiene exactamente las palabras de la pregunta y encima sube por frecuencia). Arreglo: `load_history` descarta las líneas que ejecutan `pistasho` (también `sudo pistasho`, rutas absolutas); test incluido. Nota: los `zsh -ic` de las pruebas guardan historial real del usuario; las pruebas con un shell interactivo deben usar `HISTFILE=/dev/null`. Sigue pendiente que `Ctrl-Space` no hace nada: es 2.3a, todavía no implementada (hoy solo funciona `Ctrl-G`). Calidad de `ask` aún floja en ejemplos reales (`dar permisos de ejecución` → `chmod g-x`, que los quita): es la distancia semántica de 2.9.
- 2026-11-20 · 2.3a hecha. `Ctrl-Space` abre la línea de pregunta (` ✦ ask `, en español ` ✦ pregunta `) sobre la línea del prompt, sin pantalla alternativa; Enter resuelve con `ask` (spinner `resolving…` solo si tarda más de 150 ms) y deja el mejor comando en `BUFFER` con `⚡ local · tldr/uptime · 41 ms` debajo; Esc o Ctrl-C la dejan como estaba; el texto que ya había en el prompt se precarga como pregunta; si el comando tiene marcadores se piden en un viewport corto de 7 filas (`tui::Options.start_fill`). Se eliminó el widget de `Ctrl-G` y `keys.search` (los config viejos con esa clave la ignoran); `pistasho config` ahora pregunta `keys.ask`. Nuevos: `src/prompt.rs`, `ask --prompt --format zsh --initial`, protocolo `comando NUL mensaje` (código 1 = sin coincidencia). Verificado en un zsh real dentro de tmux: sin texto previo, con texto previo, Esc, relleno, Ctrl-C en el relleno, sin coincidencias, Ctrl-C en la pregunta. Desvíos del plan: (1) el mensaje no incluye todavía `Ctrl-Space: ver N más`, porque las alternativas llegan en 2.3b y la pista prometería algo que no funciona; (2) `Tab` con la línea vacía ya abre el buscador de antes (era 2.3c): es la única forma de explorar en zsh mientras tanto, pero 2.3c sigue abierta para reescribirlo como lista `explorar` del ADR (título, `❯` de filtro); (3) el editor de la pregunta solo edita al final (sin flechas ni Ctrl-A/E), suficiente para frases cortas. El popup de tmux sigue con `pistasho search` hasta 2.3d. El README ya describe `Ctrl-Space`. Si `Ctrl-Space` choca con el cambio de método de entrada del escritorio, se cambia `keys.ask`.
