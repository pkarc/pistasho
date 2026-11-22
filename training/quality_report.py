#!/usr/bin/env python3
"""Compares the questions different models wrote (output of gen_questions.py), before any training.

    python3 training/quality_report.py gemma.jsonl qwen.jsonl --review training/out/gen/review.md [--retrieve]

Automatic checks per file: how many examples got questions (an empty list is the model saying "too
generic"), questions per example, whether the language tag matches the text, questions that still
name the tool, repeated questions, and with --retrieve how often the example the question was
written from is among the 20 candidates `pistasho ask` retrieves (needs the binary and the database,
XDG_* like pistasho). The decisive test is still the pilot fine-tune on the eval banks.

--review writes a shuffled sample with the model hidden, for a blind read, and the key next to it.
"""
import argparse
import json
import random
import re
import statistics
from pathlib import Path

from common import local_candidates_batch

ES = {"cómo", "como", "para", "el", "la", "los", "las", "un", "una", "de", "del", "en", "que", "qué", "quiero", "necesito", "puedo", "mi", "mis", "se", "con", "y", "por"}
EN = {"how", "the", "to", "my", "i", "a", "do", "what", "can", "of", "in", "and", "is", "for", "with", "on", "all", "this"}


def looks(lang, text):
    words = set(re.findall(r"[a-záéíóúñü]+", text.lower()))
    accents = bool(re.search(r"[áéíóúñ¿¡]", text.lower()))
    if lang == "es":
        return accents or len(words & ES) >= 1
    return not accents and len(words & EN) >= 1


def shape(cmd):
    return re.sub(r"\{\{.*?\}\}", "{{}}", cmd)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("files", nargs="+")
    ap.add_argument("--retrieve", action="store_true")
    ap.add_argument("--review", default=None)
    ap.add_argument("--sample", type=int, default=30)
    ap.add_argument("--seed", type=int, default=5)
    args = ap.parse_args()

    all_q = {}
    for path in args.files:
        recs = [json.loads(l) for l in open(path) if l.strip()]
        qs = [(r, lang, text) for r in recs for lang, text in r["questions"]]
        all_q[path] = (recs, qs)
        n = len(recs)
        empty = sum(1 for r in recs if not r["questions"])
        bad_lang = sum(1 for _, lang, text in qs if not looks(lang, text))
        dup = len(qs) - len({text.lower() for _, _, text in qs})
        lens = [len(text.split()) for _, _, text in qs] or [0]
        print(f"\n== {Path(path).name}")
        print(f"   {n} examples, {n - empty} with questions ({empty / max(n, 1) * 100:.0f}% empty = judged too generic), "
              f"{len(qs) / max(n - empty, 1):.1f} questions each")
        print(f"   words per question: median {statistics.median(lens):.0f}, max {max(lens)}; "
              f"language tag does not match the text: {bad_lang / max(len(qs), 1) * 100:.1f}%; repeated: {dup}")
        if args.retrieve and qs:
            sample = random.Random(args.seed).sample(qs, min(600, len(qs)))
            found = local_candidates_batch([t for _, _, t in sample], 20, 6)
            top1 = top20 = 0
            for r, _, text in sample:
                cands = found.get(" ".join(text.split()), [])
                hit = [i for i, c in enumerate(cands) if c["tool"] == r["tool"] and shape(c["command"]) == shape(r["cmd"])]
                top20 += bool(hit)
                top1 += bool(hit) and hit[0] == 0
            print(f"   retrieval of the source example (600 sampled): top-1 {top1 / len(sample) * 100:.0f}%, in 20 candidates {top20 / len(sample) * 100:.0f}%")

    if args.review:
        rng = random.Random(args.seed)
        picks = []
        for path, (_, qs) in all_q.items():
            picks += [(path, r, lang, text) for r, lang, text in rng.sample(qs, min(args.sample, len(qs)))]
        rng.shuffle(picks)
        lines = ["# Blind review: does this sound like something a person would type for that command?\n"]
        key = {}
        for i, (path, r, lang, text) in enumerate(picks, 1):
            lines.append(f"{i}. [{lang}] {text}\n   command: `{r['cmd']}`  ({r['desc'].strip()})\n")
            key[i] = Path(path).name
        Path(args.review).write_text("\n".join(lines))
        Path(args.review).with_suffix(".key.json").write_text(json.dumps(key, indent=1))
        print(f"\nreview sheet: {args.review} (+ .key.json)")


if __name__ == "__main__":
    main()
