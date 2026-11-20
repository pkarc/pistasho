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

**Las fases 0 y 1 están escritas y ya compilan con dependencias reales** (tarea 1.1 de `PLAN.md`: build, clippy, tests y CI en verde). Aún sin verificar de punta a punta: `pistasho config`, `pistasho update`, el widget de zsh y el popup de tmux (tarea 1.2).

La lista detallada de tareas, con tamaños y criterios de "lista cuando", está en `PLAN.md`. Marca las tareas con `[x]` y anota los cambios de plan en su sección de Notas.

## Estructura

```
src/
  main.rs          clap: search, query, update, add, init {zsh,tmux}, config, eval
  i18n.rs          idioma de la interfaz (en por defecto, es o auto): `tr("English", "Español")`, `PISTASHO_LANG`
  config.rs        ~/.config/pistasho/config.toml + asistente interactivo (`pistasho config`)
  db.rs            SQLite (`~/.local/share/pistasho/pistasho.db`): `entries`, `entries_fts` (FTS5, BM25, `remove_diacritics 2`), `meta`; `fts_query` con stemming ligero
  index.rs         Entry/Source; tldr desde zip → db; cheats.toml e historial se resincronizan por huella (mtime + tamaño) al abrir
  tldr.rs          parser del markdown de tldr-pages + rutas pages[.lang]/plataforma/x.md   (sin deps)
  placeholders.rs  {{marcadores}}: parse, fill, normalize_tldr ({{[-o|--opt]}} → --opt)     (sin deps)
  history.rs       ~/.zsh_history: unmetafy (0x83), formato extendido, multilínea, agregado (sin deps)
  search.rs        nucleo-matcher + bonus (herramienta exacta +80, mío +40, historial hasta +40); busca por prefijos (solo reevalúa lo que ya coincidía), en hilos, y solo en los primeros 150 caracteres de `alt`
  tui.rs           ratatui en /dev/tty: lista de candidatos (modos alternativas / explorar) + relleno de marcadores, en línea debajo del prompt. Hoy sigue siendo el buscador de `Ctrl-G`; refactor en la tarea 2.3b
  shell.rs         código que imprime `init zsh` (el widget de `Ctrl-Space`) e `init tmux` (display-popup). Hoy aún genera el widget de `Ctrl-G`; se reemplaza en la tarea 2.3a
  eval.rs          top-1 / top-3 por herramienta sobre eval/questions.toml
eval/questions.toml   40 preguntas en español con sus herramientas correctas (meta: unas 100); claves `question` / `tools`
examples/cheats.toml
```

Detalles ya resueltos que conviene no romper:
- tldr se descarga de `https://github.com/tldr-pages/tldr/releases/latest/download/tldr.zip` (todos los idiomas). `page_path` acepta prefijos de carpeta.
- Las traducciones se unen por comando normalizado: `desc` sale del primer idioma de `languages` y las demás descripciones más el resumen de la página van a `alt`, que se usa solo para buscar.
- `placeholders::fill` deja `{{x}}` intacto cuando el valor es `None` o vacío.
- El historial ignora ruido (`ls`, `cd`, `clear`…) y se queda con los `history_limit` más recientes.
- La base es derivada: `update` rehace las filas de tldr; un `SCHEMA_VERSION` distinto la reinicia. Las columnas SQL son `description` y `uses` (no `desc` / `count`, palabras reservadas).
- Historial de zsh: `HISTFILE` es una variable de zsh que casi nunca está exportada, así que el widget la pasa (`HISTFILE="$HISTFILE" command pistasho …`) y `Config::history_path` prueba `~/.zsh_history`, `~/.histfile`… si no hay variable.
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

[ui]
height = "40%"                # alto del buscador en zsh: 40% | 15 | full

[keys]
search = "^G"   # obsoleto (ADR 0001): solo se enlaza hasta que la tarea 2.3a quite el widget de Ctrl-G; luego se ignora
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
cargo run -- eval                           # precisión sobre eval/questions.toml
cargo run --release -- query --timing tar czf   # tiempos de apertura y por tecla (medir siempre en release)
cargo run -- init zsh                       # revisar el widget generado
```
