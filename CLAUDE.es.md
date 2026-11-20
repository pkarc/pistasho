# pistasho: contexto para Claude Code

[English](CLAUDE.md) · [Español](CLAUDE.es.md)

> *pista* + *sh*: la pista del comando que necesitas, sin salir de la terminal.

Herramienta en Rust para encontrar comandos de Linux desde zsh y tmux con un atajo de teclado. Repo: `pkarc/pistasho`. Licencia: MIT OR Apache-2.0.

## Principios que no se negocian

1. **Nunca ejecutar el comando sugerido.** pistasho solo lo escribe en el prompt (`BUFFER` de zle o `tmux send-keys -l` sin Enter). El Enter final siempre lo da el usuario.
2. **Lo único que se ejecuta por cuenta propia es una instalación, y siempre con confirmación explícita** que muestra el comando exacto. Solo gestores oficiales (apt, pacman, dnf, zypper), nunca `curl | sh`.
3. **Privacidad:** a un modelo remoto solo se envían la pregunta, el OS y el shell. Nunca el historial, archivos ni rutas. El filtrado de datos (por ejemplo `fc -ln 1 | grep root`) ocurre en local.
4. **La interfaz se dibuja en `/dev/tty`; stdout lleva solo el comando elegido.** Así el widget de zsh lo captura con `$(...)`.
5. **Inglés por defecto, español alineado.** La documentación, los identificadores, los comentarios y la interfaz por defecto van en inglés. La interfaz en español se elige con `language = "es"` o `PISTASHO_LANG=es`; toda cadena de cara al usuario se escribe en el sitio de uso como `tr("English", "Español")` (`src/i18n.rs`), así que una cadena nueva necesita ambas. Cada documento tiene su gemelo en español (`README.es.md`, `CLAUDE.es.md`, `PLAN.es.md`); todo cambio en uno se aplica al otro en el mismo commit.
6. **Linux primero.** macOS (brew) queda para después.
7. Para la velocidad y la calidad: **medir con `pistasho eval` antes de agregar complejidad.**

## Estado actual

**Las fases 0 y 1 están escritas y ya compilan con dependencias reales** (tarea 1.1 de `PLAN.md`: build, clippy, tests y CI en verde). Aún sin verificar de punta a punta: `pistasho config`, `pistasho update`, el widget de zsh y el popup de tmux (tarea 1.2).

La lista detallada de tareas, con tamaños y criterios de "lista cuando", está en `PLAN.md`. Marca las tareas con `[x]` y anota los cambios de plan en su sección de Notas.

## Estructura

```
src/
  main.rs          clap: search, query, update, add, init {zsh,tmux}, config, eval
  i18n.rs          idioma de la interfaz (en por defecto, es): `tr("English", "Español")`, `PISTASHO_LANG`
  config.rs        ~/.config/pistasho/config.toml + asistente interactivo (`pistasho config`)
  index.rs         Entry/Source; tldr desde zip → ~/.local/share/pistasho/index.json; cheats.toml; historial
  tldr.rs          parser del markdown de tldr-pages + rutas pages[.lang]/plataforma/x.md   (sin deps)
  placeholders.rs  {{marcadores}}: parse, fill, normalize_tldr ({{[-o|--opt]}} → --opt)     (sin deps)
  history.rs       ~/.zsh_history: unmetafy (0x83), formato extendido, multilínea, agregado (sin deps)
  search.rs        nucleo-matcher + bonus (herramienta exacta +80, mío +40, historial hasta +40)
  tui.rs           ratatui en /dev/tty, pantalla alternativa, modo búsqueda y modo relleno de marcadores
  shell.rs         código que imprime `init zsh` (widget zle) e `init tmux` (display-popup)
  eval.rs          top-1 / top-3 por herramienta sobre eval/questions.toml
eval/questions.toml   40 preguntas en español con sus herramientas correctas (meta: unas 100); claves `question` / `tools`
examples/cheats.toml
```

Detalles ya resueltos que conviene no romper:
- tldr se descarga de `https://github.com/tldr-pages/tldr/releases/latest/download/tldr.zip` (todos los idiomas). `page_path` acepta prefijos de carpeta.
- Las traducciones se unen por comando normalizado: `desc` sale del primer idioma de `languages` y las demás descripciones más el resumen de la página van a `alt`, que se usa solo para buscar.
- `placeholders::fill` deja `{{x}}` intacto cuando el valor es `None` o vacío.
- El historial ignora ruido (`ls`, `cd`, `clear`…) y se queda con los `history_limit` más recientes.
- Códigos de salida de `search`: 0 = elegido, 130 = cancelado, 2 = error.

## Diseño de la experiencia

Hay tres GIFs simulados (no capturas reales, dibujados con textos en español) que definen la interfaz deseada; los textos en inglés de la documentación en inglés son su traducción. Si están en `docs/`, úsalos como referencia:
1. **Buscador** (`Ctrl-G`): lista con etiquetas `tldr` / `mío` / `hist`, descripción y origen abajo, relleno de marcadores, popup de tmux.
2. **Modo pregunta** (`Ctrl-Space`): el prompt cambia a ` ✦ pregunta `, escribes en lenguaje natural, Enter → spinner `resolviendo…` → el comando queda en el prompt normal. Si falta la herramienta: caja amarilla con OS, gestor y `sudo apt install -y X`, con las opciones `[s]` instalar, `[a]` alternativa sin instalar y `[esc]` cancelar. Al instalar, la salida de apt y la contraseña de sudo se ven en la terminal y al final se completa el comando.
3. **Casos difíciles:** línea `⚡ local · tldr/zip · 41 ms` debajo del prompt (mensaje tipo `zle -M`); `✦ llm · haiku · 1.1 s · guardado en caché`; comando destructivo con caja roja (efecto, datos del dispositivo desde `lsblk`, señales), que queda en el prompt resaltado en rojo con `⚠ destructivo` a la derecha; dispositivos elegidos solo entre los extraíbles de `lsblk`.

Pendiente de la fase 1: los GIFs muestran el buscador **compacto debajo del prompt** (tipo `fzf --height 40%`). Hoy usa la pantalla alternativa completa. Se puede cambiar a `Viewport::Inline` de ratatui cuando lo demás funcione.

## Hoja de ruta

El detalle por tareas está en `PLAN.md`. Resumen:

### Fase 2: modo pregunta sin IA (siguiente)
- Widget `Ctrl-Space` (configurable: `keys.ask`). Cambia el prompt a modo pregunta (por ejemplo `zle recursive-edit` con un PS1 temporal, o leer la pregunta en el binario sobre `/dev/tty`). Enter llama a `pistasho ask "<pregunta>"`, el resultado va a `BUFFER` y vuelve el prompt normal.
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
- Buscador compacto (`Viewport::Inline`), empaquetado (cargo-dist, AUR), GIFs reales grabados con `vhs`, documentación.

## Configuración (`~/.config/pistasho/config.toml`)

```toml
language = "en"                 # idioma de la interfaz: en | es
languages = ["en", "es"]        # idiomas de tldr; el primero da la descripción que se muestra
platforms = ["common", "linux"]
history = true
history_limit = 5000
tldr_url = "https://github.com/tldr-pages/tldr/releases/latest/download/tldr.zip"

[keys]
search = "^G"   # zsh bindkey
ask = "^ "      # Ctrl-Space, fase 2
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
cargo run -- init zsh                       # revisar el widget generado
```
