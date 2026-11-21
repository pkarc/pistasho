# pistasho

[English](README.md) · [Español](README.es.md)

> *pista* + *sh*: la pista del comando que necesitas, sin salir de la terminal.

Presionas `Ctrl-Space` en zsh, escribes lo que quieres con tus palabras (en español o en inglés) y el mejor comando queda en tu prompt. **Nunca se ejecuta solo**: el Enter final siempre es tuyo. Busca entre las páginas de [tldr](https://tldr.sh), tu propio cheatsheet y tu historial, y todo se queda en tu máquina.

Estado: **fase 2 terminada**. Las preguntas se responden en local con BM25 sobre un índice de texto completo, y un segundo `Ctrl-Space` muestra alternativas cuando la respuesta no es la correcta. Opcionalmente un modelo local de embeddings multilingüe (e5-small, ~135 MB) se fusiona con BM25 para que una pregunta coincida por significado (`search.embeddings`, lo descarga `pistasho update`). Instalar una herramienta que falta y los avisos de riesgo ya están; Laya y el LLM de respaldo vienen más adelante. Ver [Hoja de ruta](#hoja-de-ruta).

## Instalación

```sh
cargo install --path .
pistasho config          # asistente: idiomas, atajos, modelos; descarga tldr al final
```

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
pistasho eval eval/questions.toml [--mode fuzzy|bm25|hybrid] # mide la precisión del buscador
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

## Hoja de ruta

- [x] **Fase 0** · proyecto, CI, `init zsh`
- [x] **Fase 1** · buscador: tldr (es + en), cheatsheet, historial, marcadores, popup de tmux, `eval`
- [x] **Fase 2** · modo pregunta (`Ctrl-Space`) sin IA: SQLite FTS5 (BM25) + embeddings multilingües, verificación de binarios, detección de OS, instalación con confirmación, reglas de riesgo
- [ ] **Fase 3** · Laya (ONNX) como capa de decisión opcional, solo si mejora las mediciones de `eval`
- [ ] **Fase 4** · LLM de respaldo (Anthropic / Ollama) con caché; solo envía pregunta + OS + shell
- [ ] **Fase 5** · empaquetado (AUR), demos grabadas con `vhs`

La lista detallada de tareas está en [PLAN.md](PLAN.md) (en español, igual que las decisiones de diseño en `docs/adr/`).

## Licencia

MIT o Apache-2.0, a tu elección.
