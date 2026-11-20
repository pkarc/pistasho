# pistasho

[English](README.md) · [Español](README.es.md)

> *pista* + *sh*: the hint for the command you need, without leaving the terminal.

Press `Ctrl-Space` in zsh, type what you want in plain words (English or Spanish), and the best command lands on your prompt. **It never runs by itself**: the final Enter is always yours. It searches [tldr](https://tldr.sh) pages, your own cheatsheet and your shell history, and everything stays on your machine.

Status: **phase 2 in progress**. Questions are answered locally with BM25 over a full-text index. Alternatives for a wrong answer, embeddings, installing a missing tool and risk warnings come next; Laya and the LLM fallback come later. See [Roadmap](#roadmap).

## Installation

```sh
cargo install --path .
pistasho config          # wizard: languages, keys, models; downloads tldr at the end
```

In `~/.zshrc`:

```sh
eval "$(pistasho init zsh)"
```

In `~/.tmux.conf` (popup on `prefix + g`, works in any pane, even inside ssh):

```sh
pistasho init tmux >> ~/.tmux.conf
```

## Usage

| Where | Key | What it does |
|---|---|---|
| zsh | `Ctrl-Space` | Takes over the prompt line (` ✦ ask `): type a question, `Enter` puts the best command on the prompt. Text already on the prompt becomes the question |
| question | `Enter` | Resolve; if the command has placeholders, asks for their values |
| question | `Tab` (empty line) | Explore everything: your cheats, history and tldr |
| question | `Esc` / `Ctrl-C` | Cancel, the prompt stays as it was |
| tmux | `prefix + g` | Opens the searcher in a popup and sends the command to the pane |
| list | `↑ ↓` / `Ctrl-P Ctrl-N` | Move |
| list | `Enter` | Pick; if it has placeholders, asks for their values |
| list | `Tab` | Insert the template as is, without filling |
| list | `Esc` | Cancel |
| fill-in | `Enter` / `Tab` / `Shift-Tab` | Next / skip / back |
| fill-in | `Esc` | Insert with the placeholders still missing |

`Ctrl-Space` is `keys.ask` in `config.toml` (zsh `bindkey` notation, `"^ "`). Some desktops use `Ctrl-Space` to switch the input method; change the key if that gets in the way.

Source labels: `tldr` (tldr pages), `mine` (your cheatsheet), `hist` (your zsh history).

### Subcommands

```sh
pistasho search [-q text] [--send-to PANE] [--no-fill]   # interface; prints the command
pistasho query search text [-n 10] [--timing]            # no interface, for testing; --timing prints timings
pistasho update                                           # downloads tldr and rebuilds the index
pistasho add 'fuser -k {{port}}/tcp' -d 'Kill the process on a port' -t net
pistasho init zsh | tmux
pistasho config
pistasho ask how do I compress a folder                    # best command for a question (--json -n 5: candidates)
pistasho eval eval/questions.toml [--mode fuzzy|bm25]     # measures searcher accuracy
```

### Searcher height

In zsh the lists and the placeholder fill-in open compactly below the prompt (40% of the screen by default). Change it with `[ui] height` in `config.toml` (`40%`, a number of lines, or `full`) or with `pistasho search --height`. The fill-in after a question always uses a short fixed height. The tmux popup always uses its full area.

## Language

The interface is in English by default. Set `language = "es"` in `config.toml` (or pick it in `pistasho config`) for Spanish, or `language = "auto"` to follow the system locale (`LC_ALL`, `LC_MESSAGES`, `LANG`). `PISTASHO_LANG=en|es|auto` overrides the config. The `--help` text is English only.

## Files

| Path | Contents |
|---|---|
| `~/.config/pistasho/config.toml` | Configuration (see `pistasho config`) |
| `~/.config/pistasho/cheats.toml` | Your cheatsheet ([example](examples/cheats.toml)) |
| `~/.local/share/pistasho/pistasho.db` | SQLite database: the tldr index (`pistasho update`) plus your cheats and history, re-synced when their files change |

History is read from `history_file` in the configuration, `$HISTFILE`, or the usual places (`~/.zsh_history`, `~/.histfile`, `~/.local/share/zsh/history`). **It never leaves your machine.**

## Roadmap

- [x] **Phase 0** · project, CI, `init zsh`
- [x] **Phase 1** · searcher: tldr (es + en), cheatsheet, history, placeholders, tmux popup, `eval`
- [ ] **Phase 2** · question mode (`Ctrl-Space`) without AI: SQLite FTS5 (BM25) + multilingual embeddings, binary verification, OS detection, install with confirmation, risk rules
- [ ] **Phase 3** · Laya (ONNX) as an optional decision layer, only if it improves the `eval` measurements
- [ ] **Phase 4** · LLM fallback (Anthropic / Ollama) with cache; only sends the question + OS + shell
- [ ] **Phase 5** · packaging (AUR), demos recorded with `vhs`

The detailed task list lives in [PLAN.md](PLAN.md) (written in Spanish, like the design decisions in `docs/adr/`).

## License

MIT or Apache-2.0, at your option.
