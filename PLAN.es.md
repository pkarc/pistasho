# pistasho: plan de implementación

[English](PLAN.md) · [Español](PLAN.es.md)

Cada tarea está pensada para una sesión de Claude Code: un objetivo, qué tocar y cómo saber que está lista. Hazlas en orden dentro de cada fase, salvo que se indique lo contrario. Marca `[x]` al terminar y anota en **Notas** lo que cambió respecto al plan.

Tamaños: **S** = menos de una sesión, **M** = una sesión, **L** = dos o más sesiones (conviene partirla).

Reglas para todas las tareas:
- `cargo fmt --check`, `cargo clippy --all-targets` sin warnings nuevos y `cargo test` pasan.
- Toda lógica pura nueva lleva pruebas unitarias.
- Nada ejecuta el comando sugerido (ver principios en `CLAUDE.md`).
- La documentación va en inglés por defecto; todo cambio en un documento se refleja en su gemelo `*.es.md` en el mismo commit.

---

## Fase 1 · Cierre del buscador

### [x] 1.1 Compilar y corregir · S
El código nunca se compiló con sus dependencias.
- `cargo build`, corregir errores de API (ratatui 0.29, nucleo-matcher 0.3, ureq 2, zip 2, clap 4).
- Si una versión fijada da problemas, actualizarla y anotarlo.

**Lista cuando:** compila, clippy y tests pasan, y CI queda en verde en GitHub.

### [ ] 1.2 Prueba de punta a punta · S
- `pistasho config` → `pistasho update` → `pistasho query tar` devuelve ejemplos de tar en español.
- Widget en zsh: `Ctrl-G` con texto previo, elegir, rellenar marcadores, el comando queda en el prompt y el cursor al final.
- Popup de tmux: el comando llega al panel correcto sin Enter, también dentro de una sesión ssh.
- Cancelar con Esc deja el prompt como estaba.

**Lista cuando:** todo lo anterior funciona en la máquina del usuario y los problemas encontrados están corregidos.

### [ ] 1.3 Rendimiento base · S
- Medir el tiempo desde `Ctrl-G` hasta ver resultados (cargar `index.json` + historial) y el tiempo por tecla con ~30k entradas.
- Si la carga pasa de ~150 ms: cambiar el índice a un formato binario (bincode / postcard) o adelantar la parte de SQLite de la tarea 2.1.

**Lista cuando:** hay números anotados en `docs/eval.md`, la apertura queda en menos de ~150 ms y cada tecla en menos de ~30 ms.

### [ ] 1.4 Línea base de `eval` · S
- Correr `pistasho eval` y guardar top-1 / top-3 en `docs/eval.md`.
- Ampliar `eval/preguntas.toml` hacia ~100 preguntas con comandos que el usuario realmente usa (puede ser una tarea continua).

**Lista cuando:** `docs/eval.md` tiene la línea base de la fase 1 con fecha.

---

## Fase 2 · Modo pregunta sin IA

### [ ] 2.1 Índice en SQLite + FTS5 · M
- Agregar `rusqlite` con `bundled`. Base en `~/.local/share/pistasho/pistasho.db`.
- Tablas: `entries` (cmd, desc, alt, tool, source, origin, count), tabla virtual `entries_fts` (FTS5 sobre desc, alt, tool, tags) y `meta` (versión del esquema e idiomas).
- `update` escribe en SQLite. Cheats e historial se sincronizan al abrir (comparando la fecha de modificación de los archivos).
- El buscador difuso de la fase 1 sigue igual, pero leyendo de SQLite.
- Normalizar acentos en FTS5 (`tokenize = "unicode61 remove_diacritics 2"`).

**Lista cuando:** `index.json` desaparece, el buscador funciona igual o más rápido, y una prueba confirma que "compresión" encuentra "comprimir" o "compression" vía `alt`.

### [ ] 2.2 `pistasho ask` con BM25 · M
- Subcomando `ask "<pregunta>"` que devuelve el mejor candidato por BM25 (`bm25(entries_fts)`), con `--json` para depurar (candidatos y puntuaciones).
- Quitar palabras vacías en español e inglés de la pregunta antes de buscar ("qué", "cómo", "de", "the"…).
- Subir el peso de las entradas `mío` y de las del historial que se usan seguido.
- `eval` acepta `--modo fuzzy|bm25` para comparar.

**Lista cuando:** `pistasho eval --modo bm25` supera claramente a la línea base de la 1.4 y el resultado está anotado en `docs/eval.md`.

### [ ] 2.3 Widget del modo pregunta (`Ctrl-Space`) · M
- `init zsh` agrega el widget `_pistasho_ask` en `keys.ask`.
- Experiencia del GIF 2: el prompt cambia a ` ✦ pregunta ` con una pista a la derecha (`⏎ preguntar · esc salir`). Enter resuelve y deja el comando en el prompt normal; Esc vuelve al prompt original sin cambios.
- Opción recomendada: el binario lee la pregunta en `/dev/tty` dentro de la línea (sin pantalla alternativa), muestra el spinner y devuelve el comando por stdout. Alternativa: `zle recursive-edit` con un `PS1` temporal.
- Debajo del prompt, mensaje con `zle -M`: `⚡ local · tldr/zip · 12 ms`.
- Si el comando trae marcadores, se piden igual que en el buscador.

**Lista cuando:** el flujo del GIF 2 (caso `grep root`) funciona en zsh, con y sin texto previo en el prompt.

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

**Lista cuando:** `eval --modo hybrid` supera a `bm25`, la latencia de `ask` sigue en menos de ~150 ms, y está anotado en `docs/eval.md`. Si no mejora, dejarlo apagado por defecto y anotar por qué.

### [ ] 2.10 Cierre de fase 2 · S
- `eval` final de la fase, README actualizado y un GIF real con `vhs` del modo pregunta.

---

## Fase 3 · Modelo de decisión (opcional)

> Solo se empieza si en `docs/eval.md` el top-1 híbrido está por debajo de ~85% o si decidir "ninguno sirve" con umbrales de BM25 resulta poco confiable.

### [ ] 3.1 Interfaz `Decide` + Jev / laya-serve por HTTP · M
- Trait `Decide` con `choice(estado, opciones) -> (índice, prob)` y `noul(estado, pregunta) -> prob`.
- Cliente HTTP para la API compatible con Jev, que sirve para Jev y para laya-serve; `decide.endpoint`, `decide.api_key_env`.
- `ask` usa choice entre los 10 mejores candidatos + "ninguno".
- `eval --modo hybrid+decide`.

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

### [ ] 5.1 Buscador compacto · M
- `Viewport::Inline` de ratatui debajo del prompt, como `fzf --height 40%`. Opción `ui.height` y la pantalla completa como alternativa.

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
- 2026-11-19 · Idioma: el proyecto ahora va en inglés por defecto (README, CLAUDE.md, PLAN.md). Las versiones en español se conservan como `README.es.md`, `CLAUDE.es.md` y `PLAN.es.md` y deben mantenerse alineadas. Las cadenas de la interfaz y los comentarios del código siguen en español; migración pendiente.
