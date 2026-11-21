# pistasho: contexto para Claude Code

> *pista* + *sh*: la pista del comando que necesitas, sin salir de la terminal.

Herramienta en Rust para preguntar en lenguaje natural con `Ctrl-Space` (zsh, o un popup de tmux) y tener el comando en el prompt; si no es el correcto, `Ctrl-Space` otra vez muestra las alternativas. Decisión de diseño: [ADR 0001](docs/adr/0001-question-mode-single-entry.md). Repo: `pkarc/pistasho`. Licencia: MIT OR Apache-2.0.

## Principios que no se negocian

1. **Nunca ejecutar el comando sugerido.** pistasho solo lo escribe en el prompt (`BUFFER` de zle o `tmux send-keys -l` sin Enter). El Enter final siempre lo da el usuario.
2. **Lo único que se ejecuta por cuenta propia es una instalación, y siempre con confirmación explícita** que muestra el comando exacto. Solo gestores oficiales (apt, pacman, dnf, zypper), nunca `curl | sh`.
3. **Privacidad:** a un modelo remoto solo se envían la pregunta, el OS y el shell. Nunca el historial, archivos ni rutas. El filtrado de datos (por ejemplo `fc -ln 1 | grep root`) ocurre en local.
4. **La interfaz se dibuja en `/dev/tty`; stdout lleva solo el comando elegido.** Así el widget de zsh lo captura con `$(...)`.
5. **Idioma.** El código (identificadores y comentarios) va en inglés. El README tiene dos versiones alineadas (`README.md` en inglés y `README.es.md`). La herramienta habla inglés por defecto y también español: se elige con `language = "es"` o `PISTASHO_LANG=es`, o con `language = "auto"` se detecta del idioma del sistema (`LC_ALL`, `LC_MESSAGES`, `LANG`). Toda cadena visible se escribe en el sitio de uso como `tr("English", "Español")` (`src/i18n.rs`), así que una cadena nueva necesita ambos idiomas. Las decisiones (`docs/adr/`), el plan (`PLAN.md`) y este contexto para el agente van **solo en español**, en un único archivo, sin gemelos en inglés.
6. **Linux primero.** macOS (brew) queda para después.
7. Para la velocidad y la calidad: **medir con `pistasho eval` antes de agregar complejidad.**
8. **Una sola entrada (`Ctrl-Space`).** El caso normal nunca debe costar más que pregunta + Enter.

## Estado actual

**Fase 2 en curso.** Hechas: 1.1, 1.3, 1.4; 2.1 (SQLite + FTS5), 2.2 (`ask` con BM25), 2.3a (`Ctrl-Space`), 2.3b (alternativas con un segundo `Ctrl-Space`), 2.3c (explorar con `Tab`) y 2.3d (popup de tmux en modo pregunta), 2.4 (verificación de binarios) y 2.5 (SO y paquetes) y 2.6 (instalar con confirmación) y 2.7 (alternativas sin instalar) y 2.8 (reglas de riesgo); 5.1 adelantada. Pendientes de la fase 2: 2.9 y 2.10. Falta confirmar a mano la 1.2 dentro de una sesión ssh. Línea base y mediciones en `docs/eval.md`.

La lista detallada de tareas, con tamaños y criterios de "lista cuando", está en `PLAN.md`. Marca las tareas con `[x]` y anota los cambios de plan en su sección de Notas.

## Estructura

```
src/
  main.rs          clap: search, query, ask, update, add, init {zsh,tmux}, config, doctor, eval
  i18n.rs          idioma de la interfaz (en por defecto, es o auto): `tr("English", "Español")`, `PISTASHO_LANG`
  config.rs        ~/.config/pistasho/config.toml + asistente interactivo (`pistasho config`)
  db.rs            SQLite (`~/.local/share/pistasho/pistasho.db`): `entries`, `entries_fts` (FTS5, BM25, `remove_diacritics 2`), `meta`; `fts_query` con stemming ligero
  ask.rs           `pistasho ask`: palabras vacías, BM25 (`db::fts_search_words`), boost de mío / historial, dedupe por forma del comando
  system.rs        qué tiene la máquina: `which_in` (busca en PATH sin lanzar procesos), `binaries(cmd)` (los programas que ejecuta un comando: tokenizador con comillas, `$(...)`, tuberías, redirecciones, asignaciones, palabras clave y envoltorios `sudo`/`env`/`xargs`…), `missing_binaries`
  packages.rs      SO (`/etc/os-release`: `ID`, `ID_LIKE`) → gestor (apt, pacman, dnf, zypper); tabla curada `data/packages.toml` (comando → paquete por gestor, y programas que la distro instala con otro nombre); `exists` (pregunta al gestor); `adapt` (reescribe `fd` → `fdfind`); `plan_install` (el comando de instalación, sin ejecutarlo)
  alternatives.rs  `[a]`: tabla curada `data/alternatives.toml` (programa que falta → comando hecho con herramientas base, p. ej. `tree` → `find {{dir}} -maxdepth 2 | sort`); solo para comandos que ejecutan ese único programa y si lo que ejecuta la alternativa está instalado
  risk.rs          reglas fijas de riesgo (`ok` / `caution` / `destructive`) sobre `system::commands` (programa, argumentos, redirecciones) y unos patrones de texto (bomba fork, SQL con un cliente SQL, `curl | sh`); `assess` devuelve nivel, hallazgos con su efecto y los dispositivos de bloque a los que escribe
  devices.rs       dispositivos de bloque: `is_block_device`, marcadores que son un dispositivo, `lsblk -J` (rm como bool o "0"/"1") y `removable` (solo extraíbles; los discos con particiones se listan por sus particiones)
  prompt.rs        la línea de pregunta de `Ctrl-Space`: toma la línea del prompt en /dev/tty (sin pantalla alternativa), editor simple (solo al final), spinner `resolving…` si tarda > 150 ms
  index.rs         Entry/Source; tldr desde zip → db; cheats.toml e historial se resincronizan por huella (mtime + tamaño) al abrir
  tldr.rs          parser del markdown de tldr-pages + rutas pages[.lang]/plataforma/x.md   (sin deps)
  placeholders.rs  {{marcadores}}: parse, fill, normalize_tldr ({{[-o|--opt]}} → --opt)     (sin deps)
  history.rs       ~/.zsh_history: unmetafy (0x83), formato extendido, multilínea, agregado (sin deps)
  search.rs        nucleo-matcher + bonus (herramienta exacta +80, mío +40, historial hasta +40); busca por prefijos (solo reevalúa lo que ya coincidía), en hilos, y solo en los primeros 150 caracteres de `alt`
  tui.rs           ratatui en /dev/tty (`Screen` prepara y devuelve la terminal; `confirm_install` es la caja amarilla): lista de candidatos con tres modos (`ListMode::Search` = las cuatro cajas de `pistasho search` y el popup; `Explore` y `Alternatives` = una sola caja con filtro `❯`, en línea, 11 filas) + relleno de marcadores (con `start_fill`, un viewport corto de 7 filas)
  shell.rs         código que imprime `init zsh` (el widget de `Ctrl-Space`) e `init tmux` (display-popup). Genera solo el widget de `Ctrl-Space` (`_pistasho_ask`); el de `Ctrl-G` ya no existe
  eval.rs          top-1 / top-3 por herramienta sobre eval/questions.toml; `--mode fuzzy|bm25`, `--weights`
eval/questions.toml   101 preguntas en español con sus herramientas correctas (40 originales + 61 genéricas); claves `question` / `tools`
examples/cheats.toml
```

Detalles ya resueltos que conviene no romper:
- tldr se descarga de `https://github.com/tldr-pages/tldr/releases/latest/download/tldr.zip` (todos los idiomas). `page_path` acepta prefijos de carpeta.
- Las traducciones se unen por comando normalizado: `desc` sale del primer idioma de `languages` y las demás descripciones más el resumen de la página van a `alt`, que se usa solo para buscar.
- `placeholders::fill` deja `{{x}}` intacto cuando el valor es `None` o vacío.
- El historial ignora ruido (`ls`, `cd`, `clear`…) y se queda con los `history_limit` más recientes.
- La base es derivada: `update` rehace las filas de tldr; un `SCHEMA_VERSION` distinto la reinicia. Las columnas SQL son `description` y `uses` (no `desc` / `count`, palabras reservadas).
- Historial de zsh: `HISTFILE` es una variable de zsh que casi nunca está exportada, así que el widget la pasa (`HISTFILE="$HISTFILE" command pistasho …`) y `Config::history_path` prueba `~/.zsh_history`, `~/.histfile`… si no hay variable.
- Popup de tmux: `display-popup` NO expande formatos (`#{pane_id}`) dentro de su comando; el proceso recibe el texto literal. Por eso `init tmux` genera un `run-shell -b` (que sí expande `#{pane_id}` y `#{client_name}`) que lanza `tmux display-popup` con valores ya resueltos. El `TMUX_PANE` del popup es el del propio popup, no el panel de destino. Probar siempre con un cliente tmux real (pty), no solo con `send-keys`. El popup ejecuta `pistasho ask --prompt --send-to <panel>`: la misma línea de pregunta, y como no hay `BUFFER` donde dejar la respuesta, la muestra como la lista `Alternatives` con `send: true` (la mejor seleccionada, `↑↓ see more · ⏎ send · esc cancel`); Enter (con relleno si hay marcadores) la escribe con `tmux send-keys -l`. Sin coincidencias usa `prompt::notice` (el popup se cierra con el proceso y se llevaría el mensaje). `pistasho search --send-to` sigue existiendo para scripts.
- Widget de `Ctrl-Space`: `pistasho ask --prompt --format zsh --initial "$BUFFER"` imprime `comando NUL mensaje NUL pregunta` (zsh conserva los NUL en `$(...)`); código 0 = hay comando, 1 = sin coincidencia (la salida es el mensaje), 130 = cancelado. El widget llama a `zle reset-prompt` y luego a `zle -M` (al revés se borra el mensaje). Todo se dibuja en /dev/tty; stdout solo lleva el resultado. `"^ "` y `"^@"` son la misma tecla en zsh. `keys.search` ya no existe (los config viejos con esa clave la ignoran).
- Alternativas: el widget guarda `_PISTASHO_LAST_Q` y `_PISTASHO_LAST_CMD`; si `$BUFFER` sigue siendo exactamente el último comando, `Ctrl-Space` llama a `ask --alternatives --current "$BUFFER" -- "$_PISTASHO_LAST_Q"`; si no, es una pregunta nueva. Se borran en `zle-line-finish` (`add-zle-hook-widget`). La lista parte de los 20 mejores candidatos de BM25 (`ListMode::Alternatives`, índices en las entradas cargadas) con el comando actual seleccionado (`placeholders::matches_template` lo encuentra aunque ya esté relleno); con el filtro vacío muestran los candidatos, al escribir se busca en todo con el buscador difuso. `ask --explore` abre `ListMode::Explore`.
- Binarios de un comando (`system::binaries`): los `{{marcadores}}` son una sola pieza (su `|` y sus espacios no son sintaxis de shell; sin eso salían programas falsos como `id}}`), los envoltorios (`sudo`, `env`, `xargs`, `nohup`, `time`, `nice`…) se saltan por lo que ejecutan, los builtins del shell nunca cuentan como faltantes, y los alias o funciones del usuario no se ven desde fuera (un comando que es alias saldría como faltante). `ask` rellena `Candidate.missing`; `--json` lo expone como `available` / `missing` y el mensaje bajo el prompt avisa `⚠ ncdu not installed`. No cambia el orden de los candidatos.
- Paquetes (`packages.rs`, `data/packages.toml`): la tabla solo lista las excepciones; lo que no está se supone empaquetado con su propio nombre y el gestor lo confirma antes de proponerlo (`apt-cache policy` mira la línea `Candidate:` porque sale con 0 aunque no exista; `pacman -Si` y `dnf -q info` con su código de salida; `zypper what-provides`, no `info`, porque openSUSE llama `python313` al paquete y ofrece `python3` como capacidad). Los comandos de instalación no piden confirmación otra vez (`-y`, `--noconfirm`, `--non-interactive`) porque pistasho ya preguntó mostrando el comando exacto. **Al cambiar la tabla, validarla** con `pistasho doctor --table` en un contenedor de cada familia (`docker run --rm -v ~/.cargo/bin/pistasho:/usr/local/bin/pistasho:ro archlinux ...`; en Arch hace falta `pacman -Sy` antes y en openSUSE `zypper refresh`): así se encontraron `docker` → `moby-engine` y `7z` → `7zip` en Fedora, y los nombres de openSUSE.
- `adapt` reescribe los programas que faltan pero existen con otro nombre en esa distro (solo `fd`, `bat` en Debian/Ubuntu y `python` → `python3`); se aplica al comando final (ya relleno) en `ask_on_prompt`, `ask_alternatives`, explorar y el popup, y el mensaje lo dice (`fd → fdfind`). `Candidate.missing` se calcula sobre el comando adaptado, así que no marca como faltante lo que se puede ejecutar con la reescritura.
- Instalación (`offer_install` en `main.rs`, `tui::confirm_install`, `packages::run_install`): es lo único que pistasho ejecuta por su cuenta. Solo tras una tecla explícita (`y`, o `s` en español; **Enter no confirma**, hay un test), con el comando exacto en la caja, sin shell (los nombres de paquete se validan: `a;b`, `--yes`… nunca llegan al comando) y todo o nada (si un paquete no existe no se ofrece el plan). El hijo hereda `/dev/tty` como stdin/stdout/stderr, no el stdout del proceso (dentro del widget es el pipe de `$(...)`). Después se vuelve a comprobar el binario. No existe ninguna opción que instale sin preguntar; `install.offer = false` apaga la oferta.
- Salidas de `ask --prompt --format zsh`: 0 = comando, 1 = sin coincidencia, 5 = la instalación falló (la salida es el mensaje), 130 = cancelado; el cuarto campo es `1` si algo escribió en la terminal (el gestor de paquetes).
- zle tras una instalación: **no usar `zle -I`** (ni solo ni con `reset-prompt`): medido en tmux, el prompt se pierde o pisa la salida de `apt`. Lo que redibuja bien es `zle reset-prompt` a secas con el cursor en una línea en blanco; y como `reset-prompt` sube tantas líneas como tenga el prompt menos una, el widget (`_pistasho_room`) deja esas líneas en blanco antes (cuenta con `rows=("${(@f)${(%)PS1}}")`; `${#...}` de un escalar cuenta caracteres). Probado con prompts de 1, 2 y 3 líneas.
- Cómo probar la instalación sin tocar la máquina: un `sudo` y un `apt` falsos en una carpeta al principio del `PATH` (el `apt` falso crea el binario) más `XDG_CONFIG_HOME` / `XDG_DATA_HOME` temporales con un atajo de prueba; y, para `apt` de verdad, `docker run -it ubuntu:24.04` con un `sudo` de relleno y la base copiada, manejado desde un panel de tmux.
- Alternativas (`alternatives.rs`): reemplazan el comando entero (con sus propios `{{marcadores}}`, que se piden igual) y las opciones del original no se trasladan. Solo si el comando ejecuta **un único programa** y es el que falta (con una tubería no se sabría qué parte sustituir), y solo si todo lo que ejecuta la alternativa está instalado. La caja las muestra en una fila `Instead:`; si no hay paquete para instalar, la caja solo ofrece `[a]` y `y` no hace nada. Al elegirla el mensaje es `⚡ alternative · no install · N ms` (N = lo que tardó en resolver, no lo que tardó el usuario en pulsar) y en el popup se salta la lista. `ask --json` expone `alternative`.
- Riesgo (`risk.rs`): `finalize` en `main.rs` es el paso común de todo comando antes de entregarlo (respuesta, alternativas, explorar, popup y `pistasho search`): renombres, binarios que faltan, y `risk::assess`. Un comando `destructive` muestra la caja roja (`tui::confirm_risk`: `⏎` lo deja en el prompt, `e` lo edita en una línea y lo vuelve a evaluar, `esc` lo descarta; aquí **Enter es la tecla segura** porque dejarlo escrito no ejecuta nada). `caution` no abre caja: marca la línea en amarillo con el efecto. Las reglas NO leen SQL salvo que haya un cliente SQL en el comando (`grep 'drop table' x` es una búsqueda), `rm -rf ./build` es `caution` y no `destructive`, y `chmod -R` solo es destructivo sobre `/` y los directorios del sistema. Protocolo del widget: `comando NUL mensaje NUL pregunta NUL printed NUL riesgo NUL etiqueta`. El widget marca la línea con `RPROMPT` (`⚠ destructivo`, en rojo) y `region_highlight` con `memo=pistasho` (así no pisa a otros plugins como zsh-syntax-highlighting); lo quita al editar o limpiar la línea (`zle-line-pre-redraw`) y al aceptarla (`zle-line-finish`).
- Selector de dispositivos (`tui::Fill`, `devices.rs`): si un marcador es un dispositivo (`{{/dev/sdX}}`, `{{dispositivo}}`, `{{usb_device}}`…) la lista de **solo extraíbles** de `lsblk` ocupa el lugar del campo de texto y no se puede teclear otro (para que un error o un Enter apresurado no apunte `mkfs` al disco del sistema); los discos no extraíbles salen en una nota (`sda hidden: not removable`). Si no hay ninguno extraíble, se escribe a mano. `Fill::new` solo llama a `lsblk` si la plantilla tiene un marcador de dispositivo.
- Pruebas de punta a punta: `python3 scripts/e2e.py [nombre…]` (hermético: XDG propios, `sudo` / `apt` / `apt-cache` / `lsblk` falsos; necesita tmux, zsh y una máquina tipo Debian). Cubre pregunta, instalación (con prompts de 1, 2 y 3 líneas), alternativa, caja roja, selector de USB, caution/ok y el popup de tmux con un cliente real en un pty. **Correrlo antes de cada commit que toque el widget, `tui.rs` o `main.rs`**: los tests unitarios no ven lo que muestra la terminal. Los scripts sueltos en /tmp se perdieron una vez; no volver a dejarlos ahí.
- Códigos de salida de `search`: 0 = elegido, 130 = cancelado, 2 = error.

## Diseño de la experiencia

Hay GIFs simulados (no capturas reales, dibujados con textos en español) que definen la interfaz deseada; la herramienta real mostrará esos textos en inglés por defecto (y en español con `language = "es"`). Están en `docs/`:
1. **Modo pregunta y alternativas** (`docs/pistasho-modo-pregunta.gif`, [ADR 0001](docs/adr/0001-question-mode-single-entry.md)), cuatro pasos:
   1. `Ctrl-Space`: el prompt cambia a ` ✦ pregunta ` (pista `⏎ preguntar · tab explorar · esc salir`), escribes, Enter → spinner `resolviendo…` → el mejor comando queda en el prompt normal, con `⚡ local · tldr/fc · Ctrl-Space: ver 5 más` debajo.
   2. ¿No era ese? `Ctrl-Space` otra vez (con el `BUFFER` sin editar) abre, compacta debajo del prompt, `otras opciones · «<pregunta>»`: un campo de filtro `❯` y filas `tldr` / `mío` / `hist` + comando con marcadores resaltados. `↑↓` mover, `Enter` reemplaza, `Esc` deja el actual.
   3. Escribes para filtrar, eliges y Enter reemplaza el comando (`✓ reemplazado · mío/cheats.toml`). Si editas el `BUFFER` o ejecutas el comando, el siguiente `Ctrl-Space` es una pregunta nueva.
   4. `Ctrl-Space` + `Tab` con el campo vacío: `explorar · tus atajos, historial y tldr` (el antiguo buscador: mío → historial reciente → tldr, filtro difuso, `Tab` inserta la plantilla sin rellenar).
2. **Instalar y alternativa** (`docs/pistasho-casos.gif`, caso 4 y la caja de instalación): si falta la herramienta, caja amarilla con OS, gestor y `sudo apt install -y X`, con las opciones `[s]` instalar, `[a]` alternativa sin instalar y `[esc]` cancelar. Al instalar, la salida de apt y la contraseña de sudo se ven en la terminal y al final se completa el comando.
3. **Casos difíciles** (`docs/pistasho-casos.gif`): línea `⚡ local · tldr/zip · 41 ms` debajo del prompt (mensaje tipo `zle -M`); `✦ llm · haiku · 1.1 s · guardado en caché`; comando destructivo con caja roja (efecto, datos del dispositivo desde `lsblk`, señales; opciones `[⏎]` dejar en el prompt, `[e]` editar, `[esc]` cancelar), que queda en el prompt resaltado en rojo con `⚠ destructivo` a la derecha; dispositivos elegidos solo entre los extraíbles de `lsblk`. Los puntajes `laya …` de ese GIF son de la fase 3; antes de eso la línea lleva el puntaje de BM25 / híbrido.

La lista de candidatos va **en línea debajo del prompt** (tipo `fzf --height 40%`) con `Viewport::Inline` de ratatui, ya implementado antes de tiempo como tarea 5.1 y reutilizado por la tarea 2.3b; `ui.height` / `--height` aceptan `40%`, un número de líneas o `full`. El popup de tmux siempre usa su área completa. Ojo: dentro del widget stdout es el pipe de `$(...)` y ratatui/crossterm consultan la posición del cursor por stdout, así que `tui::run` apunta stdout al tty mientras corre (lo restaura `Guard` antes de imprimir el comando).

## Hoja de ruta

El detalle por tareas está en `PLAN.md`. Resumen:

### Fase 2: modo pregunta sin IA (siguiente)
- Widget `Ctrl-Space` (configurable: `keys.ask`), la única entrada. Cambia el prompt a modo pregunta (por ejemplo `zle recursive-edit` con un PS1 temporal, o leer la pregunta en el binario sobre `/dev/tty`). Enter llama a `pistasho ask "<pregunta>"`, el mejor comando va a `BUFFER` y vuelve el prompt normal. Un segundo `Ctrl-Space` sobre el resultado sin editar abre la lista de alternativas de la misma pregunta (`ask --alternatives`); `Tab` con la pregunta vacía abre la lista de explorar (`ask --explore`). `ask` devuelve los N mejores candidatos con su origen.
- **Índice en SQLite + FTS5** (rusqlite con `bundled`) para BM25 sobre `desc` + `alt` + etiquetas. También guarda la caché pregunta → comando y las estadísticas de uso.
- **Embeddings multilingües** (familia `multilingual-e5-small`, ~120 MB cuantizado, ONNX vía `ort` o `fastembed`). Vectores precalculados en `update`; al preguntar solo se calcula el de la pregunta. Búsqueda híbrida BM25 + embeddings con fusión por rangos (RRF). Resuelve sinónimos y pregunta en español contra descripción en inglés.
- **Verificación de binarios:** `command -v` / buscar en `PATH`.
- **Detección de OS** con `/etc/os-release` (`ID`, `ID_LIKE`) → gestor de paquetes.
- **Comando ≠ paquete:** tabla curada (`rg`→`ripgrep`; `fd`→`fd-find` en Debian/Ubuntu, donde el binario queda como `fdfind`; `bat`→`batcat` en Debian, etc.) y **verificación con el gestor** antes de proponer (`apt-cache policy`, `pacman -Si`, `dnf info`).
- **Instalación:** caja de confirmación; se ejecuta en primer plano con stdin/stdout en la tty para que sudo pida la contraseña. Al terminar, verificar el binario otra vez y completar el comando.
- **Opción `[a]`:** alternativa con binarios ya instalados (por ejemplo `tree` → `find src -maxdepth 2 | sort`). En esta fase sale de una tabla curada.
- **Reglas fijas de riesgo:** `rm -rf`, `dd of=/dev/`, `mkfs`, `chmod -R`/`chown -R` sobre `/`, `> /dev/sd*`, fork bombs, `:(){`. Aviso rojo y nunca auto-relleno silencioso.
- Medir con `pistasho eval` contra la línea base de la fase 1 y guardar los números en el README o en un `docs/eval.md`.

### Fase 3: modelo de decisión (opcional, solo si `eval` lo justifica)
- Interfaz `Decide` con tres implementaciones: Laya local (ONNX), laya-serve (HTTP compatible con Jev) y Jev (API de TypeSafe).
- Usos: **choice** entre los ~10 mejores candidatos + "ninguno" (decide si hace falta el LLM), **noul** "¿es destructivo?" y **noul** "¿la alternativa cumple la petición?".
- Datos investigados (noviembre de 2026): Jev es **no generativo** (choice / score / noul con probabilidades), alojado, a $0.042 por millón de tokens de entrada. Laya es abierto (Apache 2.0), el checkpoint multilingüe es mmBERT-base de 322M parámetros, y el inglés es ModernBERT-large de 421M. **Los "33 ms" son en GPU T4; en CPU son unos 370–380 ms el modelo grande** (el multilingüe ~2× más rápido), con 0.5 GB de RAM en int8 y 1.7 GB en fp32. **Sin fine-tuning su precisión es baja** (0.36 en una prueba independiente) y viene sin calibrar. Es una base para especializar, no algo listo para usar.
- Por eso necesita un **daemon** (`pistasho daemon`, socket unix en `$XDG_RUNTIME_DIR`) que mantenga el modelo en memoria, y **fine-tuning + calibración** con nuestro conjunto de prueba ampliado.
- Criterio para descartarlo: si la búsqueda híbrida ya acierta el top-1 en un 85–90% de los casos, si Laya ajustado no mejora 5–10 puntos, o si tarda más de ~300 ms en la máquina del usuario.
- El usuario tiene un servidor Proxmox (CPU Intel) donde podría correr laya-serve.

### Fase 4: LLM de respaldo
- Interfaz `Generate`: Anthropic (por defecto `claude-haiku-5-5`, key desde la variable de `llm.api_key_env`) y Ollama (`llm.endpoint`, `llm.model`).
- Respuesta en JSON estructurado: `{command, binaries[], package?, risk, explanation}`. Validar siempre con el paso de binarios y riesgo de la fase 2.
- Caché en SQLite; la segunda vez que se hace la misma pregunta va por la vía local (`⚡`).
- Solo se envían pregunta + OS + shell.

### Fase 5: pulido
- Empaquetado (cargo-dist, AUR), GIFs reales grabados con `vhs`, documentación.

## Configuración (`~/.config/pistasho/config.toml`)

```toml
language = "en"                 # idioma de la interfaz: en | es | auto (detecta el del sistema)
languages = ["en", "es"]        # idiomas de tldr; el primero da la descripción que se muestra
platforms = ["common", "linux"]
history = true
history_limit = 5000
tldr_url = "https://github.com/tldr-pages/tldr/releases/latest/download/tldr.zip"

[install]
offer = true                  # la caja de instalar / alternativa; false la apaga. Nunca instala sin preguntar

[ui]
height = "40%"                # alto del buscador en zsh: 40% | 15 | full

[keys]
ask = "^ "      # Ctrl-Space: la única entrada (fase 2)
tmux = "g"      # prefix + g

[decide]        # fase 3: none | laya | laya-serve | jev
provider = "none"

[llm]           # fase 4: none | anthropic | ollama
provider = "none"
```

El usuario pidió que todo sea configurable **desde la primera ejecución**, con el asistente (`pistasho config`) o editando el archivo. Al agregar opciones nuevas, súmalas también al asistente.

## Comandos útiles

```sh
cargo build && cargo clippy --all-targets && cargo test
cargo run -- query comprimir carpeta        # búsqueda sin interfaz
cargo run -- eval                           # precisión (BM25) sobre eval/questions.toml
cargo run -- eval --mode fuzzy              # la línea base de la fase 1
cargo run -- ask comprimir una carpeta      # la mejor respuesta; --json -n 5 para ver candidatos
cargo run --release -- query --timing tar czf   # tiempos de apertura y por tecla (medir siempre en release)
cargo run -- init zsh                       # revisar el widget generado
```
