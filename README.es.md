# pistasho

[English](README.md) · [Español](README.es.md)

> *pista* + *sh*: la pista del comando que necesitas, sin salir de la terminal.

Presionas `Ctrl-Space` en zsh, escribes lo que quieres con tus palabras (en español o en inglés) y el mejor comando queda en tu prompt. **Nunca se ejecuta solo**: el Enter final siempre es tuyo. Busca entre las páginas de [tldr](https://tldr.sh), tu propio cheatsheet y tu historial, y todo se queda en tu máquina.

Estado: **fases 0–4 terminadas, fase 5 (empaquetado y pulido) en curso**; todavía no hay ninguna versión publicada. Las preguntas se responden en local con BM25 más un modelo opcional de embeddings multilingüe, un segundo `Ctrl-Space` muestra alternativas, una herramienta que falta se puede instalar tras una confirmación explícita, los comandos peligrosos llevan aviso, y un LLM de respaldo opcional responde lo que el índice local no puede. Ver [Privacidad](#privacidad) y la [Hoja de ruta](#hoja-de-ruta).

## Instalación

```sh
cargo install --path .   # desde un clon de este repositorio
pistasho config          # asistente: idiomas, atajos, modelos; descarga tldr al final
```

Las instalaciones empaquetadas (binarios de los releases de GitHub para x86_64 y aarch64, `cargo install pistasho` y un paquete de AUR) están preparadas pero todavía no se publican; ver [docs/release.md](docs/release.md). El primer `pistasho update` descarga tldr y el modelo de embeddings (~135 MB) y calcula los vectores (unos 5 minutos; se puede interrumpir y reanuda). Con `search.embeddings = false` se salta el modelo y `ask` usa solo BM25.

En `~/.zshrc`:

```sh
eval "$(pistasho init zsh)"
```

En `~/.tmux.conf` (popup con `prefix + g`, funciona en cualquier panel, incluso dentro de ssh):

```sh
pistasho init tmux >> ~/.tmux.conf
```

## Uso

| Dónde | Tecla | Qué hace |
|---|---|---|
| zsh | `Ctrl-Space` | Toma la línea del prompt (` ✦ pregunta `): escribes una pregunta, `Enter` deja el mejor comando en el prompt. El texto que ya había en el prompt pasa a ser la pregunta |
| zsh | `Ctrl-Space` otra vez | ¿No era ese? Mientras el prompt siga con la respuesta, abre **otras opciones** para la misma pregunta: una línea de filtro y los candidatos. Escribir busca en todo, `Enter` reemplaza el comando, `Esc` lo deja |
| pregunta | `Enter` | Resolver; si el comando tiene marcadores, pide sus valores |
| pregunta | `Tab` (línea vacía) | Explorar todo: tus atajos, historial y tldr |
| pregunta | `Esc` / `Ctrl-C` | Cancelar, el prompt queda como estaba |
| tmux | `prefix + g` | Abre la misma línea de pregunta en un popup. La respuesta se muestra con las demás candidatas (`↑↓` ver más, `Enter` envía, `Tab` en una línea vacía explora) y el comando se escribe en el panel sin Enter, también por ssh |
| lista | `↑ ↓` / `Ctrl-P Ctrl-N` | Mover |
| lista | `Enter` | Elegir; si tiene marcadores, pide sus valores |
| lista | `Tab` | Insertar la plantilla tal cual, sin rellenar |
| lista | `Esc` | Cancelar |
| relleno | `Enter` / `Tab` / `Shift-Tab` | Siguiente / saltar / volver |
| relleno | `Esc` | Insertar con los marcadores que falten |

`Ctrl-Space` es `keys.ask` en `config.toml` (notación `bindkey` de zsh, `"^ "`). Algunos escritorios usan `Ctrl-Space` para cambiar el método de entrada; cambia la tecla si te estorba.

Etiquetas de origen: `tldr` (páginas de tldr), `mío` (tu cheatsheet), `hist` (tu historial de zsh).

### Subcomandos

```sh
pistasho search [-q texto] [--send-to PANE] [--no-fill]   # interfaz; imprime el comando
pistasho query texto de búsqueda [-n 10] [--timing]        # sin interfaz, para pruebas; --timing imprime tiempos
pistasho update                                             # descarga tldr (y el modelo de embeddings) y reconstruye el índice
pistasho add 'fuser -k {{puerto}}/tcp' -d 'Matar el proceso de un puerto' -t red
pistasho doctor [programas…] [--table]                      # SO, gestor de paquetes y cómo conseguir un programa que falta
pistasho init zsh | tmux
pistasho config
pistasho ask cómo comprimo una carpeta                     # mejor comando para una pregunta (--json -n 5: candidatos)
pistasho ask --llm renombrar todos los jpeg a jpg           # pregunta directo al LLM (necesita `[llm]`)
pistasho cache list | clear [pregunta] | save <pregunta>   # respuestas del LLM en caché; guarda una como atajo propio
pistasho eval eval/questions.toml [--mode fuzzy|bm25|hybrid] [--confidence] # mide la precisión del buscador
```

### Alto del buscador

En zsh las listas y el relleno de marcadores abren compactos debajo del prompt (40% de la pantalla por defecto). Cámbialo con `[ui] height` en `config.toml` (`40%`, un número de líneas o `full`) o con `pistasho search --height`. El relleno tras una pregunta siempre usa una altura corta fija. El popup de tmux siempre usa su área completa.

## Idioma

La interfaz va en inglés por defecto. Pon `language = "es"` en `config.toml` (o elígelo en `pistasho config`) para español, o `language = "auto"` para seguir el idioma del sistema (`LC_ALL`, `LC_MESSAGES`, `LANG`). `PISTASHO_LANG=en|es|auto` tiene prioridad sobre la configuración. El texto de `--help` solo está en inglés.

## Archivos

| Ruta | Contenido |
|---|---|
| `~/.config/pistasho/config.toml` | Configuración (ver `pistasho config`) |
| `~/.config/pistasho/cheats.toml` | Tu cheatsheet ([ejemplo](examples/cheats.toml)) |
| `~/.local/share/pistasho/models/` | El modelo de embeddings (`pistasho update`; se omite con `search.embeddings = false`). `PISTASHO_EMBEDDINGS=0` lo apaga en un comando |
| `~/.local/share/pistasho/pistasho.db` | Base SQLite: el índice de tldr (`pistasho update`) más tus atajos e historial, que se resincronizan cuando sus archivos cambian |

El historial se lee de `history_file` en la configuración, `$HISTFILE` o los sitios habituales (`~/.zsh_history`, `~/.histfile`, `~/.local/share/zsh/history`). **No sale de tu máquina.**

## LLM de respaldo (opcional)

Apagado por defecto (`llm.provider = "none"`): nada sale de tu máquina. Para encenderlo, corre `pistasho config`, elige `anthropic` y pon tu API key en la variable de entorno que indica (`ANTHROPIC_API_KEY` por defecto; pistasho nunca guarda la key). Entonces, cuando nada local coincide o la mejor coincidencia es dudosa (mejor puntaje menor que `llm.threshold`, 0.20 por defecto; con `llm.when = "nomatch"` solo cuando nada coincide), la pregunta va al modelo y su comando llega al prompt como cualquier otro: se revisa si faltan programas y el riesgo, y **nunca se ejecuta solo**. Las respuestas se guardan en caché (`pistasho cache list | clear | save`), así que repetir la misma pregunta no cuesta nada, y si la llamada falla se muestra la respuesta local. `pistasho doctor` muestra si está listo.

## Privacidad

- **Tu historial, tus archivos y tus rutas nunca salen de tu máquina.** La búsqueda, los embeddings y las reglas de riesgo corren en local.
- El único uso de red es `pistasho update` (tldr desde GitHub y el modelo de embeddings desde Hugging Face, comprobado contra un SHA-256 fijo) y el LLM de respaldo opcional.
- El LLM de respaldo, si lo enciendes, envía **solo** la pregunta que escribiste, el nombre del SO, el gestor de paquetes y el nombre del shell al proveedor. Tus preguntas podrían contener cosas que prefieras no compartir: léelas antes de preguntar.
- La caché de las respuestas del LLM y tus preguntas viven en la base local (`~/.local/share/pistasho/pistasho.db`).
- Lo único que pistasho ejecuta por su cuenta es la instalación de un paquete, y solo después de que pulses `y` con el comando exacto a la vista (solo gestores oficiales, nunca `curl | sh`).

## Cómo contribuir

- **Atajos y tldr:** tus comandos propios van en `~/.config/pistasho/cheats.toml` (`pistasho add`). Para comandos que sirven a todos, envíalos a [tldr-pages](https://github.com/tldr-pages/tldr): pistasho lee esas páginas, en inglés y en español.
- **Preguntas para `eval`:** `pistasho eval` mide cuántas veces sale primero la herramienta correcta. Agrega preguntas reales (las que te fallaron) a [`eval/questions.toml`](eval/questions.toml) como `question = "..."` más las `tools` que cuentan como correctas, y corre `pistasho eval --mode hybrid`. `eval/questions-synth.toml` tiene preguntas generadas y se mantiene aparte a propósito.
- Antes de enviar un cambio: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`, y `PISTASHO_BIN=$PWD/target/release/pistasho python3 scripts/e2e.py` si tocaste el widget, `tui.rs` o `main.rs`. Las notas de diseño y el plan están en español ([CLAUDE.md](CLAUDE.md), [PLAN.md](PLAN.md), `docs/adr/`).

## Hoja de ruta

- [x] **Fase 0** · proyecto, CI, `init zsh`
- [x] **Fase 1** · buscador: tldr (es + en), cheatsheet, historial, marcadores, popup de tmux, `eval`
- [x] **Fase 2** · modo pregunta (`Ctrl-Space`) sin IA: SQLite FTS5 (BM25) + embeddings multilingües, verificación de binarios, detección de OS, instalación con confirmación, reglas de riesgo
- ~~**Fase 3**~~ · Laya (ONNX) como capa de decisión opcional: **se decidió no construirla**; no mejoró las mediciones de `eval` y exigiría un modelo local y un daemon (ver `docs/eval.md`)
- [x] **Fase 4** · LLM de respaldo (Anthropic) con caché; se pregunta solo cuando nada local es seguro; solo envía pregunta + OS + shell
- [ ] **Fase 5** · empaquetado (workflow de releases, archivos de AUR y crates.io preparados, sin publicar), demos reales con `vhs` (guiones en `docs/demo/`, sin grabar), documentación, changelog

La lista detallada de tareas está en [PLAN.md](PLAN.md) (en español, igual que las decisiones de diseño en `docs/adr/`).

## Licencia

MIT o Apache-2.0, a tu elección.
