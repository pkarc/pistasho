# pistasho: context for Claude Code

[English](CLAUDE.md) · [Español](CLAUDE.es.md)

> *pista* + *sh*: the hint for the command you need, without leaving the terminal.

A Rust tool to find Linux commands from zsh and tmux with a keyboard shortcut. Repo: `pkarc/pistasho`. License: MIT OR Apache-2.0.

## Non-negotiable principles

1. **Never run the suggested command.** pistasho only writes it to the prompt (zle `BUFFER` or `tmux send-keys -l` without Enter). The final Enter is always the user's.
2. **The only thing run on its own is an install, and always with explicit confirmation** showing the exact command. Official package managers only (apt, pacman, dnf, zypper), never `curl | sh`.
3. **Privacy:** only the question, the OS and the shell are sent to a remote model. Never history, files or paths. Data filtering (e.g. `fc -ln 1 | grep root`) happens locally.
4. **The UI is drawn on `/dev/tty`; stdout carries only the chosen command.** That way the zsh widget captures it with `$(...)`.
5. **English by default, Spanish kept aligned.** Docs, code identifiers, comments and the default UI are in English. The Spanish UI is selected with `language = "es"` or `PISTASHO_LANG=es`; every user-facing string is written at the call site as `tr("English", "Español")` (`src/i18n.rs`), so a new string needs both. Every doc has a Spanish twin (`README.es.md`, `CLAUDE.es.md`, `PLAN.es.md`); any change to one must be applied to the other in the same commit.
6. **Linux first.** macOS (brew) comes later.
7. For speed and quality: **measure with `pistasho eval` before adding complexity.**

## Current state

**Phases 0 and 1 are written and now compile with real dependencies** (task 1.1 of `PLAN.md`: build, clippy, tests and CI all green). Not yet verified end to end: `pistasho config`, `pistasho update`, the zsh widget and the tmux popup (task 1.2).

The detailed task list, with sizes and "done when" criteria, is in `PLAN.md`. Mark tasks `[x]` and log plan changes in its Notes section.

## Structure

```
src/
  main.rs          clap: search, query, update, add, init {zsh,tmux}, config, eval
  i18n.rs          UI language (en default, es): `tr("English", "Español")`, `PISTASHO_LANG`
  config.rs        ~/.config/pistasho/config.toml + interactive wizard (`pistasho config`)
  index.rs         Entry/Source; tldr from zip → ~/.local/share/pistasho/index.json; cheats.toml; history
  tldr.rs          tldr-pages markdown parser + pages[.lang]/platform/x.md paths   (no deps)
  placeholders.rs  {{placeholders}}: parse, fill, normalize_tldr ({{[-o|--opt]}} → --opt)     (no deps)
  history.rs       ~/.zsh_history: unmetafy (0x83), extended format, multiline, aggregation (no deps)
  search.rs        nucleo-matcher + bonuses (exact tool +80, mine +40, history up to +40)
  tui.rs           ratatui on /dev/tty: compact inline viewport (zsh) or alternate screen (tmux popup), search mode and placeholder fill mode
  shell.rs         code printed by `init zsh` (zle widget) and `init tmux` (display-popup)
  eval.rs          top-1 / top-3 per tool over eval/questions.toml
eval/questions.toml   40 Spanish questions with their correct tools (goal: about 100); keys `question` / `tools`
examples/cheats.toml
```

Already-settled details worth not breaking:
- tldr is downloaded from `https://github.com/tldr-pages/tldr/releases/latest/download/tldr.zip` (all languages). `page_path` accepts folder prefixes.
- Translations are joined by normalized command: `desc` comes from the first language in `languages`; the other descriptions plus the page summary go to `alt`, which is used only for searching.
- `placeholders::fill` leaves `{{x}}` untouched when the value is `None` or empty.
- History ignores noise (`ls`, `cd`, `clear`…) and keeps the most recent `history_limit` entries.
- `search` exit codes: 0 = chosen, 130 = cancelled, 2 = error.

## Experience design

Three simulated GIFs (not real captures, drawn with Spanish strings) define the desired interface; the English strings used in the docs are their translations. If they are in `docs/`, use them as reference:
1. **Searcher** (`Ctrl-G`): list with `tldr` / `mine` / `hist` labels, description and origin below, placeholder fill-in, tmux popup.
2. **Question mode** (`Ctrl-Space`): the prompt changes to ` ✦ ask `, you type in natural language, Enter → `resolving…` spinner → the command is left on the normal prompt. If the tool is missing: a yellow box with OS, package manager and `sudo apt install -y X`, with options `[y]` install (`[s]` in the Spanish UI), `[a]` alternative without installing and `[esc]` cancel. While installing, apt's output and the sudo password prompt show in the terminal and the command is completed at the end.
3. **Hard cases:** a `⚡ local · tldr/zip · 41 ms` line below the prompt (a `zle -M`-style message); `✦ llm · haiku · 1.1 s · saved to cache`; a destructive command with a red box (effect, device data from `lsblk`, signals), left on the prompt highlighted in red with `⚠ destructive` on the right; devices chosen only among removable ones from `lsblk`.

The searcher is **compact below the prompt** in the zsh widget (like `fzf --height 40%`, task 5.1 done early) via ratatui's `Viewport::Inline`; `ui.height` / `--height` take `40%`, a line count or `full`. The tmux popup always uses its full area. Gotcha: inside the widget stdout is the `$(...)` pipe, and ratatui/crossterm query the cursor position through stdout, so `tui::run` points stdout at the tty while it runs (restored by `Guard` before the command is printed).

## Roadmap

Task-level detail is in `PLAN.md`. Summary:

### Phase 2: question mode without AI (next)
- `Ctrl-Space` widget (configurable: `keys.ask`). Switches the prompt to question mode; Enter calls `pistasho ask "<question>"`, the result goes to `BUFFER` and the normal prompt returns.
- **SQLite + FTS5 index** (rusqlite `bundled`) for BM25 over `desc` + `alt` + tags. Also stores the question → command cache and usage stats.
- **Multilingual embeddings** (`multilingual-e5-small` family, ~120 MB quantized, ONNX via `ort` or `fastembed`). Vectors precomputed in `update`; only the question's vector is computed at ask time. Hybrid BM25 + embeddings search with rank fusion (RRF). Handles synonyms and a Spanish question against an English description.
- **Binary verification:** `command -v` / search in `PATH`.
- **OS detection** via `/etc/os-release` (`ID`, `ID_LIKE`) → package manager.
- **Command ≠ package:** curated table (`rg`→`ripgrep`; `fd`→`fd-find` on Debian/Ubuntu, where the binary becomes `fdfind`; `bat`→`batcat` on Debian, etc.) and **verification with the package manager** before proposing (`apt-cache policy`, `pacman -Si`, `dnf info`).
- **Install:** confirmation box; runs in the foreground with stdin/stdout on the tty so sudo can ask for the password. When done, verify the binary again and complete the command.
- **Option `[a]`:** alternative using already-installed binaries (e.g. `tree` → `find src -maxdepth 2 | sort`). In this phase it comes from a curated table.
- **Fixed risk rules:** `rm -rf`, `dd of=/dev/`, `mkfs`, `chmod -R`/`chown -R` on `/`, `> /dev/sd*`, fork bombs, `:(){`. Red warning and never a silent auto-fill.
- Measure with `pistasho eval` against the phase 1 baseline and record the numbers in `docs/eval.md`.

### Phase 3: decision model (optional, only if `eval` justifies it)
- `Decide` interface with three implementations: local Laya (ONNX), laya-serve (Jev-compatible HTTP) and Jev (TypeSafe API).
- Uses: **choice** among the ~10 best candidates + "none" (decides whether the LLM is needed), **noul** "is it destructive?" and **noul** "does the alternative satisfy the request?".
- Researched data (November 2026): Jev is **non-generative** (choice / score / noul with probabilities), hosted, at $0.042 per million input tokens. Laya is open (Apache 2.0); the multilingual checkpoint is mmBERT-base at 322M parameters, and the English one is ModernBERT-large at 421M. **The "33 ms" figure is on a T4 GPU; on CPU the large model takes about 370–380 ms** (the multilingual one ~2× faster), with 0.5 GB of RAM in int8 and 1.7 GB in fp32. **Without fine-tuning its accuracy is low** (0.36 in an independent test) and it comes uncalibrated. It is a base to specialize, not something ready to use.
- That is why it needs a **daemon** (`pistasho daemon`, unix socket in `$XDG_RUNTIME_DIR`) that keeps the model in memory, plus **fine-tuning + calibration** with our extended test set.
- Criteria to drop it: if hybrid search already hits top-1 in 85–90% of cases, if tuned Laya does not improve by 5–10 points, or if it takes more than ~300 ms on the user's machine.
- The user has a Proxmox server (Intel CPU) where laya-serve could run.

### Phase 4: LLM fallback
- `Generate` interface: Anthropic (default `claude-haiku-5-5`, key from the `llm.api_key_env` variable) and Ollama (`llm.endpoint`, `llm.model`).
- Structured JSON response: `{command, binaries[], package?, risk, explanation}`. Always validate with the phase 2 binary and risk steps.
- SQLite cache; the second time the same question is asked it goes through the local path (`⚡`).
- Only the question + OS + shell are sent.

### Phase 5: polish
- Packaging (cargo-dist, AUR), real GIFs recorded with `vhs`, documentation.

## Configuration (`~/.config/pistasho/config.toml`)

```toml
language = "en"                 # UI language: en | es
languages = ["en", "es"]        # tldr languages; the first provides the displayed description
platforms = ["common", "linux"]
history = true
history_limit = 5000
tldr_url = "https://github.com/tldr-pages/tldr/releases/latest/download/tldr.zip"

[ui]
height = "40%"                # zsh searcher height: 40% | 15 | full

[keys]
search = "^G"   # zsh bindkey
ask = "^ "      # Ctrl-Space, phase 2
tmux = "g"      # prefix + g

[decide]        # phase 3: none | laya | laya-serve | jev
provider = "none"

[llm]           # phase 4: none | anthropic | ollama
provider = "none"
```

The user asked for everything to be configurable **from the first run**, through the wizard (`pistasho config`) or by editing the file. When adding new options, add them to the wizard too.

## Useful commands

```sh
cargo build && cargo clippy --all-targets && cargo test
cargo run -- query comprimir carpeta        # search without the interface
cargo run -- eval                           # accuracy over eval/questions.toml
cargo run -- init zsh                       # inspect the generated widget
```
