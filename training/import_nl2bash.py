#!/usr/bin/env python3
"""Human-written English descriptions of bash commands (NL2Bash) as tool-level training questions.

NL2Bash (Lin et al., LREC 2018; the data directory is MIT licensed, the repository code is GPL and is
not used) pairs ~12 600 descriptions written by bash programmers with the one-liners they describe.
A one-liner is not one of tldr's examples, so the label is the TOOL (and the other tools it pipes
through): any tldr example of those tools is a right answer.

Two corrections to the raw data, which is not what a person asks:
  * `find` is 60% of it: at most --per-tool pairs of each first tool are kept.
  * the descriptions are technical and often long: only 3 to 22 words, no more than 4 programs.

Cite the paper if a model trained on this is published (docs/training.md).

    python3 training/import_nl2bash.py DIR_WITH_all.nl_and_all.cm --out training/out/nl2bash.jsonl
"""
import argparse
import json
import os
import random
import shlex
import sqlite3
import sys
from collections import Counter
from pathlib import Path

WRAPPERS = {"sudo", "env", "time", "nohup", "xargs", "command", "exec", "nice", "watch"}
SEPARATORS = {"|", "||", "&&", ";", "&", "(", ")", "{", "}", "!", "then", "do", "done", "fi", "else"}


def programs(command):
    """The programs a one-liner runs, in order (wrappers and variable assignments skipped)."""
    lex = shlex.shlex(command, posix=True, punctuation_chars=True)
    lex.whitespace_split = False
    try:
        tokens = list(lex)
    except ValueError:
        return []
    out, expect = [], True
    for tok in tokens:
        if tok in SEPARATORS or set(tok) <= set("|&;()<>"):
            expect = True
            continue
        if not expect:
            continue
        if tok in WRAPPERS or ("=" in tok and not tok.startswith("-") and tok.split("=")[0].isidentifier()):
            continue
        if tok.startswith("-") or tok.startswith("$"):
            expect = False
            continue
        out.append(os.path.basename(tok))
        expect = False
    return list(dict.fromkeys(out))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("dir")
    ap.add_argument("--out", required=True)
    ap.add_argument("--per-tool", type=int, default=120)
    ap.add_argument("--db", default=str(Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local/share")) / "pistasho/pistasho.db"))
    ap.add_argument("--seed", type=int, default=3)
    args = ap.parse_args()

    con = sqlite3.connect(f"file:{args.db}?mode=ro", uri=True)
    known = {r[0] for r in con.execute("select distinct tool from entries where source='tldr'")}
    con.close()
    nl = [l.strip() for l in open(Path(args.dir) / "all.nl", encoding="utf-8")]
    cm = [l.strip() for l in open(Path(args.dir) / "all.cm", encoding="utf-8")]
    rng = random.Random(args.seed)
    pairs = list(zip(nl, cm))
    rng.shuffle(pairs)
    seen, kept, per_tool, skipped = set(), [], Counter(), Counter()
    for text, command in pairs:
        words = text.split()
        progs = [p for p in programs(command) if p in known]
        if not 3 <= len(words) <= 22:
            skipped["length"] += 1
        elif not progs:
            skipped["no tldr tool"] += 1
        elif len(programs(command)) > 4:
            skipped["long pipeline"] += 1
        elif text.lower() in seen:
            skipped["duplicate"] += 1
        elif per_tool[progs[0]] >= args.per_tool:
            skipped["tool cap"] += 1
        else:
            seen.add(text.lower())
            per_tool[progs[0]] += 1
            kept.append({"question": text, "lang": "en", "tools": progs})
    Path(args.out).parent.mkdir(parents=True, exist_ok=True)
    with open(args.out, "w") as f:
        for r in kept:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")
    print(f"{len(kept)} kept of {len(pairs)}; skipped: {dict(skipped)}", file=sys.stderr)
    print("most common first tools:", per_tool.most_common(8), file=sys.stderr)


if __name__ == "__main__":
    main()
