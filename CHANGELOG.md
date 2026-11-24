# Changelog

[English](CHANGELOG.md) · [Español](CHANGELOG.es.md)

Nothing is released yet. This is what the first version, `0.1.0`, will contain.

## [0.1.0] — unreleased

### Added
- **Question mode.** `Ctrl-Space` in zsh (or `prefix + g` in a tmux popup, also over ssh) turns a question in English or Spanish into a command on your prompt. It never runs by itself.
- **Alternatives and exploring.** A second `Ctrl-Space` on the unedited answer lists other options for the same question; `Tab` on an empty question explores your cheats, history and tldr.
- **Search.** tldr pages (English + Spanish), your own `cheats.toml`, and your zsh history, ranked with BM25 over a SQLite FTS5 index, a Spanish synonym table, one tldr example per tool before repeats, and an optional multilingual embedding model (`multilingual-e5-small`) fused by rank.
- **Checks on every command:** programs that are not installed (and an offer to install them with an explicit `y` and the exact command shown), a curated table of command → package for apt, pacman, dnf and zypper, alternatives made of base tools, fixed risk rules with a red confirmation box, and a picker that offers only removable block devices.
- **LLM fallback (optional, off by default).** Anthropic, asked when nothing local matches or the best match is shaky; sends only the question, the OS name, the package manager and the shell name; answers are cached (`pistasho cache`).
- **Decision model (optional, off by default).** Jev (TypeSafe, hosted) re-orders the best tldr candidates and says when none fits (top-1 52% → 83% on 101 questions, 48% → 76% on 287 generated ones), and adds a second opinion on risk next to the fixed rules (21 of 29 destructive commands caught instead of 17, no false alarm in the sample). Only your question and public tldr text are sent, never your own commands, history or filled-in values.
- `pistasho risk <command>` and `pistasho eval --decide`.
- `pistasho eval` with top-1, top-3, recall@10 and a confidence table; a question bank of 101 real and 287 generated questions.
- `pistasho doctor`, `pistasho config` (wizard), English and Spanish interface.

### Measured
- On 101 questions, top-1 / top-3 went from 28% / 41% (phase 1 fuzzy matching) to 51% / 74% with the hybrid search. See `docs/eval.md` (written in Spanish) for every measurement behind each decision.

### Decided not to build
- A local decision model (Laya) and a local LLM (Ollama): they need a resident service and hundreds of MB of RAM for one tool, and the general rerankers tried did not help. Laya was fine-tuned in 23 variants on an RTX 5090 and none beat the local order (best top-1 54% / 44% against 53% / 46%; see `docs/eval.md`). Only hosted providers are supported.
