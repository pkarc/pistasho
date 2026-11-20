# pistasho

> *pista* + *sh*: la pista del comando que necesitas, sin salir de la terminal.

Presionas `Ctrl-G` en zsh, buscas entre las páginas de [tldr](https://tldr.sh), tu propio cheatsheet y tu historial, eliges, rellenas los `{{marcadores}}` y el comando queda en tu prompt. **Nunca se ejecuta solo**: el Enter final siempre es tuyo.

Estado: **fase 1** (buscador). El modo pregunta en lenguaje natural, Laya y el LLM de respaldo vienen después; ver [Hoja de ruta](#hoja-de-ruta).

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
| zsh | `Ctrl-G` | Abre el buscador con lo que ya escribiste como búsqueda inicial |
| tmux | `prefix + g` | Abre el buscador en un popup y envía el comando al panel |
| buscador | `↑ ↓` / `Ctrl-P Ctrl-N` | Mover |
| buscador | `Enter` | Elegir; si tiene marcadores, pide sus valores |
| buscador | `Tab` | Insertar la plantilla tal cual, sin rellenar |
| buscador | `Esc` | Cancelar |
| relleno | `Enter` / `Tab` / `Shift-Tab` | Siguiente / saltar / volver |
| relleno | `Esc` | Insertar con los marcadores que falten |

Etiquetas de origen: `tldr` (páginas de tldr), `mío` (tu cheatsheet), `hist` (tu historial de zsh).

### Subcomandos

```sh
pistasho search [-q texto] [--send-to PANE] [--no-fill]   # interfaz; imprime el comando
pistasho query texto de búsqueda [-n 10]                    # sin interfaz, para pruebas
pistasho update                                             # descarga tldr y reconstruye el índice
pistasho add 'fuser -k {{puerto}}/tcp' -d 'Matar el proceso de un puerto' -t red
pistasho init zsh | tmux
pistasho config
pistasho eval eval/preguntas.toml                           # mide la precisión del buscador
```

## Archivos

| Ruta | Contenido |
|---|---|
| `~/.config/pistasho/config.toml` | Configuración (ver `pistasho config`) |
| `~/.config/pistasho/cheats.toml` | Tu cheatsheet ([ejemplo](examples/cheats.toml)) |
| `~/.local/share/pistasho/index.json` | Índice de tldr generado por `pistasho update` |

El historial se lee de `history_file` en la configuración, `$HISTFILE` o `~/.zsh_history`. **No sale de tu máquina.**

## Hoja de ruta

- [x] **Fase 0** · proyecto, CI, `init zsh`
- [x] **Fase 1** · buscador: tldr (es + en), cheatsheet, historial, marcadores, popup de tmux, `eval`
- [ ] **Fase 2** · modo pregunta (`Ctrl-Space`) sin IA: SQLite FTS5 (BM25) + embeddings multilingües, verificación de binarios, detección de OS, instalación con confirmación, reglas de riesgo
- [ ] **Fase 3** · Laya (ONNX) como capa de decisión opcional, solo si mejora las mediciones de `eval`
- [ ] **Fase 4** · LLM de respaldo (Anthropic / Ollama) con caché; solo envía pregunta + OS + shell
- [ ] **Fase 5** · empaquetado (AUR), demos grabadas con `vhs`

## Licencia

MIT o Apache-2.0, a tu elección.
