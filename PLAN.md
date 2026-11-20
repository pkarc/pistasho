# pistasho: implementation plan

[English](PLAN.md) · [Español](PLAN.es.md)

Each task is meant to fit one Claude Code session: one goal, what to touch and how to know it is done. Do them in order within each phase unless stated otherwise. Mark `[x]` when finished and note in **Notes** whatever changed from the plan.

Sizes: **S** = less than one session, **M** = one session, **L** = two or more sessions (worth splitting).

Rules for every task:
- `cargo fmt --check`, `cargo clippy --all-targets` with no new warnings, and `cargo test` pass.
- All new pure logic comes with unit tests.
- Nothing runs the suggested command (see the principles in `CLAUDE.md`).
- Docs are English by default; every doc change is mirrored in its `*.es.md` twin in the same commit.

---

## Phase 1 · Searcher wrap-up

### [x] 1.1 Build and fix · S
The code was never compiled with its dependencies.
- `cargo build`, fix API errors (ratatui 0.29, nucleo-matcher 0.3, ureq 2, zip 2, clap 4).
- If a pinned version causes trouble, upgrade it and note it.

**Done when:** it compiles, clippy and tests pass, and CI is green on GitHub.

### [ ] 1.2 End-to-end test · S
- `pistasho config` → `pistasho update` → `pistasho query tar` returns tar examples in Spanish.
- zsh widget: `Ctrl-G` with existing text, pick, fill placeholders, the command stays on the prompt with the cursor at the end.
- tmux popup: the command reaches the right pane without Enter, also inside an ssh session.
- Cancelling with Esc leaves the prompt as it was.

**Done when:** all of the above works on the user's machine and the problems found are fixed.

### [ ] 1.3 Baseline performance · S
- Measure the time from `Ctrl-G` to seeing results (loading `index.json` + history) and the time per keystroke with ~30k entries.
- If loading goes over ~150 ms: switch the index to a binary format (bincode / postcard) or pull forward the SQLite part of task 2.1.

**Done when:** the numbers are recorded in `docs/eval.md`, opening takes under ~150 ms and each keystroke under ~30 ms.

### [ ] 1.4 `eval` baseline · S
- Run `pistasho eval` and save top-1 / top-3 in `docs/eval.md`.
- Grow `eval/preguntas.toml` toward ~100 questions with commands the user really uses (can be an ongoing task).

**Done when:** `docs/eval.md` has the phase 1 baseline with a date.

---

## Phase 2 · Question mode without AI

### [ ] 2.1 SQLite + FTS5 index · M
- Add `rusqlite` with `bundled`. Database at `~/.local/share/pistasho/pistasho.db`.
- Tables: `entries` (cmd, desc, alt, tool, source, origin, count), virtual table `entries_fts` (FTS5 over desc, alt, tool, tags) and `meta` (schema version and languages).
- `update` writes to SQLite. Cheats and history are synced on open (by comparing file modification times).
- The phase 1 fuzzy searcher stays the same, but reads from SQLite.
- Normalize accents in FTS5 (`tokenize = "unicode61 remove_diacritics 2"`).

**Done when:** `index.json` is gone, the searcher works the same or faster, and a test confirms that "compresión" finds "comprimir" or "compression" via `alt`.

### [ ] 2.2 `pistasho ask` with BM25 · M
- `ask "<question>"` subcommand that returns the best candidate by BM25 (`bm25(entries_fts)`), with `--json` for debugging (candidates and scores).
- Remove Spanish and English stop words from the question before searching ("qué", "cómo", "de", "the"…).
- Raise the weight of `mío` entries and of frequently used history entries.
- `eval` accepts `--modo fuzzy|bm25` to compare.

**Done when:** `pistasho eval --modo bm25` clearly beats the 1.4 baseline and the result is recorded in `docs/eval.md`.

### [ ] 2.3 Question-mode widget (`Ctrl-Space`) · M
- `init zsh` adds the `_pistasho_ask` widget on `keys.ask`.
- GIF 2 experience: the prompt changes to ` ✦ pregunta ` with a hint on the right (`⏎ preguntar · esc salir`). Enter resolves and leaves the command on the normal prompt; Esc returns to the original prompt unchanged.
- Recommended option: the binary reads the question on `/dev/tty` inline (no alternate screen), shows the spinner and returns the command on stdout. Alternative: `zle recursive-edit` with a temporary `PS1`.
- Below the prompt, a `zle -M` message: `⚡ local · tldr/zip · 12 ms`.
- If the command has placeholders, they are asked for just like in the searcher.

**Done when:** the GIF 2 flow (`grep root` case) works in zsh, with and without text already on the prompt.

### [ ] 2.4 Binary verification · S
- `system::which(bin)` module that searches `PATH` without spawning processes.
- Extract the binaries of a command: first word, words after `|`, `&&`, `;`, `$(`, `xargs`, skipping `sudo` / `env` / `VAR=x` assignments.
- `ask` marks each candidate as available or missing.

**Done when:** there are tests for `sudo tar …`, `ss -tlnp | grep :80`, `find . | xargs rg x` and `FOO=1 make`.

### [ ] 2.5 OS and package detection · M
- Read `/etc/os-release` (`ID`, `ID_LIKE`, `PRETTY_NAME`) → manager: apt, pacman, dnf or zypper.
- Curated table `data/packages.toml` of command → package per family, including the odd cases: `rg`→`ripgrep`; `fd`→`fd-find` on Debian, where the binary is `fdfind`; `bat`→`bat`, with binary `batcat` on Debian; `ncdu`, `tree`, `jq`, `htop`, `btop`, `delta`→`git-delta`…
- Verify the package exists before proposing it: `apt-cache policy X`, `pacman -Si X`, `dnf info X`, `zypper info X`.
- If the installed binary has a different name (`fdfind`), rewrite the command or warn.

**Done when:** tested on Ubuntu and Arch (containers are fine), with tests for the table and the os-release parser.

### [ ] 2.6 Install flow · M
- GIF 2 confirmation box: command, system, `Instalar: sudo apt install -y X`, options `[s]` install, `[a]` alternative, `[esc]` cancel.
- On confirm: run the install in the foreground inheriting the tty (sudo asks for the password and apt's output is visible). When done, verify the binary again, show `✓ X instalado` and complete the command.
- On failure: show the error and leave the prompt untouched.
- Never install without confirmation, nor with `--yes` in a default configuration.

**Done when:** the GIF 2 `ncdu` case works end to end on a machine where ncdu is missing.

### [ ] 2.7 Alternatives without installing · S
- Curated table of alternatives using base tools: `tree`→`find {{dir}} -maxdepth 2 | sort`, `ncdu`→`du -h -d1 {{dir}} | sort -h`, `rg`→`grep -rn`, `fd`→`find -name`, `jq .`→`python3 -m json.tool`, `htop`→`top`…
- The `[a]` option uses the table and verifies that the alternative's binaries exist.

**Done when:** the GIF 3 `tree` case works.

### [ ] 2.8 Risk rules · M
- `risk` module with patterns: `rm -rf` (especially on `/`, `~` or `*`), `dd … of=/dev/…`, `mkfs*`, `wipefs`, `shred`, `chmod/chown -R` on `/`, `> /dev/sd*`, fork bomb, `:(){`, `git push --force` to main, `DROP TABLE`…
- Levels: `ok`, `cuidado` (caution), `destructivo` (destructive).
- If a placeholder is a device (`{{dispositivo}}`, `/dev/sdX`): offer only the removable devices from `lsblk -J -o NAME,MODEL,SIZE,FSTYPE,MOUNTPOINT,RM`.
- On `destructivo`, the GIF 3 red box. The command stays on the prompt with `⚠ destructivo` in `RPROMPT` and highlighted (`region_highlight`).
- Applies to every command coming out of `ask`, and to the searcher too.

**Done when:** there are tests per pattern, including obvious false positives (`rm -rf ./build` is `cuidado`, not `destructivo`), and the GIF 3 USB case works.

### [ ] 2.9 Multilingual embeddings · L (split into 2.9a / 2.9b)
- **2.9a:** choose the model (`multilingual-e5-small` family, quantized) and the execution route (`fastembed` or `ort` directly). Measure RAM, size and per-question latency on the user's machine. Download it in `update`; do not bundle it in the binary.
- **2.9b:** precompute the vectors of all entries in `update` and store them in SQLite as a blob. When asking, compute the question's vector, cosine similarity (brute force over ~30k vectors is enough), and fuse with BM25 by ranks (RRF, k=60).
- `search.embeddings = true|false` option in the configuration and in the wizard.

**Done when:** `eval --modo hybrid` beats `bm25`, `ask` latency stays under ~150 ms, and it is recorded in `docs/eval.md`. If it does not improve, leave it off by default and note why.

### [ ] 2.10 Phase 2 wrap-up · S
- Final phase `eval`, updated README and a real `vhs` GIF of question mode.

---

## Phase 3 · Decision model (optional)

> Only start if, in `docs/eval.md`, hybrid top-1 is below ~85% or if deciding "none fits" with BM25 thresholds proves unreliable.

### [ ] 3.1 `Decide` interface + Jev / laya-serve over HTTP · M
- `Decide` trait with `choice(state, options) -> (index, prob)` and `noul(state, question) -> prob`.
- HTTP client for the Jev-compatible API, which serves both Jev and laya-serve; `decide.endpoint`, `decide.api_key_env`.
- `ask` uses choice among the 10 best candidates + "none".
- `eval --modo hybrid+decide`.

**Done when:** it works against laya-serve (for example on the user's Proxmox) and is measured in `eval`.

### [ ] 3.2 Training data · M
- Extend the test set with synthetic (question, candidates, correct) triples generated from tldr and reviewed. Split training and test.

### [ ] 3.3 Laya fine-tuning and calibration · L
- Fine-tune the multilingual checkpoint and calibrate the temperature. Export to ONNX int8.

**Done when:** it beats hybrid-without-Laya by 5–10 points on the test set.

### [ ] 3.4 Local Laya with a daemon · L
- `pistasho daemon`: unix socket at `$XDG_RUNTIME_DIR/pistasho.sock`, loads the model once, starts itself on the first call and shuts down after a period of inactivity.
- `ort` in Rust, no Python.

**Done when:** the first cold call takes under 2 s and later ones under ~300 ms on CPU. If that cannot be achieved, only the laya-serve route stays.

### [ ] 3.5 Risk and alternatives with noul · S
- Combine the fixed rules from 2.8 with `noul("is it destructive?")`. The rules always win; the model only adds warnings.
- `noul("does the alternative satisfy the request?")` for the `[a]` option.

### [ ] 3.6 Decision · S
- With the numbers in hand, decide whether Laya is enabled by default, optional, or dropped. Record it in `docs/eval.md`.

---

## Phase 4 · LLM fallback

### [ ] 4.1 `Generate` interface + Anthropic · M
- Call with structured output: `{command, binaries[], package?, risk, explanation}`.
- System prompt with the OS, the package manager, the shell and "one-line command, run nothing". **Never** include history, paths or files.
- Key from the `llm.api_key_env` variable; short timeout and a clear message on failure.

**Done when:** the GIF 3 `.jpeg → .jpg` case works.

### [ ] 4.2 Ollama · S
- Same interface against `llm.endpoint` / `llm.model`, with JSON format.

### [ ] 4.3 When to call the LLM · S
- Call it only if the local route found nothing reliable: BM25 / hybrid under a threshold, or "none" from the decision model.
- Show `✦ llm · model · 1.1 s` below the prompt.

### [ ] 4.4 Validation and cache · S
- Every LLM command goes through 2.4 (binaries), 2.5/2.6 (install) and 2.8 (risk) before reaching the prompt.
- Cache normalized question → command in SQLite. A repeated question resolves as `⚡ local · caché`.
- Option to save an LLM command that worked for you into `cheats.toml`.

**Done when:** the second time the same question is asked, the LLM is not called.

---

## Phase 5 · Polish and launch

### [ ] 5.1 Compact searcher · M
- ratatui `Viewport::Inline` below the prompt, like `fzf --height 40%`. `ui.height` option, with full screen as the alternative.

### [ ] 5.2 Packaging · M
- cargo-dist for GitHub releases (Linux x86_64 / aarch64 binaries), AUR package and `cargo install pistasho` (publish the crate).

### [ ] 5.3 Real demos · S
- Replace the simulated GIFs with `vhs` recordings (`.tape` files in `docs/`).

### [ ] 5.4 Documentation and launch · S
- Complete README, a privacy section, how to contribute cheats and questions to `eval`, and a CHANGELOG.

---

## Notes

<!-- Record decisions or plan changes here, with a date. -->

- 2026-11-19 · 1.1: it compiled on the first try with the pinned versions, with no API errors or version changes. Only adjustment: the `unnecessary_sort_by` clippy warning in `history.rs`. `Cargo.lock` is now versioned (it is a binary). CI green. `cargo install` puts the binary in `~/.cargo/bin`, which was not in `PATH`.
- 2026-11-19 · Language: the project is now English by default (README, CLAUDE.md, PLAN.md). The Spanish versions are kept as `README.es.md`, `CLAUDE.es.md` and `PLAN.es.md` and must stay aligned. UI strings and code comments are still Spanish; migration pending.
