# pistasho

[English](README.md) · [Español](README.es.md)

> *pista* + *sh*: the hint for the command you need, without leaving the terminal.

Press `Ctrl-Space` in zsh, type what you want in plain words (English or Spanish), and the best command lands on your prompt. **It never runs by itself**: the final Enter is always yours. It searches [tldr](https://tldr.sh) pages, your own cheatsheet and your shell history, and everything stays on your machine.

Status: **phases 0–4 done, phase 5 (packaging and polish) in progress**; nothing is released yet. Questions are answered locally with BM25 plus an optional multilingual embedding model, a second `Ctrl-Space` shows alternatives, a missing tool can be installed after an explicit confirmation, risky commands get a warning, and an optional LLM fallback answers what the local index cannot. See [Privacy](#privacy) and the [Roadmap](#roadmap).

## Installation

```sh
cargo install --path .   # from a clone of this repository
pistasho config          # wizard: languages, keys, models; downloads tldr at the end
```

Packaged installs (GitHub release binaries for x86_64 and aarch64, `cargo install pistasho`, an AUR package) are prepared but not published yet; see [docs/release.md](docs/release.md). The first `pistasho update` downloads tldr and the embedding model (~135 MB) and computes the vectors (about 5 minutes, and it can be interrupted and resumed). With `search.embeddings = false` it skips the model and `ask` uses BM25 only.

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
| zsh | `Ctrl-Space` again | Not the one? While the prompt still holds the answer, opens **other options** for the same question: a filter line and the candidates. Typing searches everything, `Enter` replaces the command, `Esc` keeps it |
| question | `Enter` | Resolve; if the command has placeholders, asks for their values |
| question | `Tab` (empty line) | Explore everything: your cheats, history and tldr |
| question | `Esc` / `Ctrl-C` | Cancel, the prompt stays as it was |
| tmux | `prefix + g` | Opens the same question line in a popup. The answer is shown with the other candidates (`↑↓` see more, `Enter` sends, `Tab` on an empty line explores) and the command is typed into the pane without Enter, also over ssh |
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
pistasho update                                           # downloads tldr (and the embedding model) and rebuilds the index
pistasho add 'fuser -k {{port}}/tcp' -d 'Kill the process on a port' -t net
pistasho doctor [programs…] [--table]                     # OS, package manager, and how to get a missing program
pistasho init zsh | tmux
pistasho config
pistasho ask how do I compress a folder                    # best command for a question (--json -n 5: candidates)
pistasho ask --llm rename all jpeg to jpg                   # ask the LLM directly (needs `[llm]`)
pistasho cache list | clear [question] | save <question>   # the LLM's cached answers; save one as your own cheat
pistasho eval eval/questions.toml [--mode fuzzy|bm25|hybrid] [--confidence] # measures searcher accuracy
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
| `~/.local/share/pistasho/models/` | The embedding model (`pistasho update`; skipped with `search.embeddings = false`). `PISTASHO_EMBEDDINGS=0` turns it off for one command |
| `~/.local/share/pistasho/pistasho.db` | SQLite database: the tldr index (`pistasho update`) plus your cheats and history, re-synced when their files change |

History is read from `history_file` in the configuration, `$HISTFILE`, or the usual places (`~/.zsh_history`, `~/.histfile`, `~/.local/share/zsh/history`). **It never leaves your machine.**

## LLM fallback (optional)

Off by default (`llm.provider = "none"`): nothing leaves your machine. To turn it on, run `pistasho config` and pick `anthropic`, and put your API key in the environment variable it names (`ANTHROPIC_API_KEY` by default; pistasho never stores the key). Then, when nothing local matches or the best match is shaky (best score under `llm.threshold`, 0.20 by default; `llm.when = "nomatch"` asks only when nothing matches), the question goes to the model and its command lands on the prompt like any other: it is checked for missing programs and risk, and **it never runs by itself**. Answers are cached (`pistasho cache list | clear | save`), so asking the same thing again costs nothing, and a failed call falls back to the local answer. `pistasho doctor` shows whether it is ready.

## Privacy

- **Your history, files and paths never leave your machine.** Searching, the embeddings and the risk rules all run locally.
- The only network use is `pistasho update` (tldr from GitHub, and the embedding model from Hugging Face, checked against a pinned SHA-256), and the optional LLM fallback.
- The LLM fallback, if you turn it on, sends **only** the question you typed, the OS name, the package manager and the shell name to the provider. Your questions can still contain things you would rather not share, so read them before asking.
- The cache of the LLM's answers and your questions live in the local database (`~/.local/share/pistasho/pistasho.db`).
- The only thing pistasho runs by itself is a package installation, and only after you press `y` with the exact command shown (official package managers only, never `curl | sh`).

## Contributing

- **Cheats and tldr:** your own commands go in `~/.config/pistasho/cheats.toml` (`pistasho add`). For commands everybody could use, send them to [tldr-pages](https://github.com/tldr-pages/tldr): pistasho reads those pages, in English and Spanish.
- **Questions for `eval`:** `pistasho eval` measures how often the right tool comes out on top. Add real questions (the ones that failed for you) to [`eval/questions.toml`](eval/questions.toml) as `question = "..."` plus the `tools` that count as correct, and run `pistasho eval --mode hybrid`. `eval/questions-synth.toml` holds generated questions and is kept apart on purpose.
- Before sending a change: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`, and `PISTASHO_BIN=$PWD/target/release/pistasho python3 scripts/e2e.py` if you touched the widget, `tui.rs` or `main.rs`. The design notes and the plan are in Spanish ([CLAUDE.md](CLAUDE.md), [PLAN.md](PLAN.md), `docs/adr/`).

## Roadmap

- [x] **Phase 0** · project, CI, `init zsh`
- [x] **Phase 1** · searcher: tldr (es + en), cheatsheet, history, placeholders, tmux popup, `eval`
- [x] **Phase 2** · question mode (`Ctrl-Space`) without AI: SQLite FTS5 (BM25) + multilingual embeddings, binary verification, OS detection, install with confirmation, risk rules
- ~~**Phase 3**~~ · Laya (ONNX) as an optional decision layer: **decided not to build**; it did not improve the `eval` measurements and would need a local model and daemon (see `docs/eval.md`)
- [x] **Phase 4** · LLM fallback (Anthropic) with cache; asked only when nothing local is sure; only sends the question + OS + shell
- [ ] **Phase 5** · packaging (release workflow, AUR and crates.io files prepared, not published), real demos with `vhs` (scripts in `docs/demo/`, not recorded yet), documentation, changelog

The detailed task list lives in [PLAN.md](PLAN.md) (written in Spanish, like the design decisions in `docs/adr/`).

## License

MIT or Apache-2.0, at your option.
