#!/usr/bin/env python3
"""A random sample of the training rows that come from some sources (train.meta.jsonl says which), with the whole validation file.

    python3 training/subset_by_source.py data-meta 40000 data-gemma40k gen-gemma gen-gemma-b nl2bash
    python3 training/subset_by_source.py data-meta2 40000 data-had40k gen-gemma gen-gemma-b nl2bash tldr-en tldr-es tldr-mix had

The word `disagree` keeps only rows where the local order's first candidate is NOT a right tool (and the answer is not "none"):
the only thing a model needs to learn to add something is when to contradict the local order. `balanced` takes n/2 of those
and n/2 rows where the local order is right (and not "none"), as the control. Both need `had` too.

The word `had` keeps only rows whose exact example was among the retrieved candidates (nothing injected; needs the
`had` field that make_data.py writes), the distribution closest to real use.
"""
import json
import random
import shutil
import sys
from pathlib import Path

src, n, dst, keep = Path(sys.argv[1]), int(sys.argv[2]), Path(sys.argv[3]), set(sys.argv[4:])
only_had = "had" in keep
mode = "disagree" if "disagree" in keep else "balanced" if "balanced" in keep else None
keep -= {"had", "disagree", "balanced"}
pool = {"disagree": [], "agree": [], "all": []}
for l, m in zip(open(src / "train.jsonl"), open(src / "train.meta.jsonl")):
    meta = json.loads(m)
    if meta["src"] not in keep or (only_had and not meta["had"]):
        continue
    pool["all"].append(l)
    if not meta["none"]:
        pool["agree" if meta["loc_tool"] else "disagree"].append(l)
rng = random.Random(7)
if mode == "disagree":
    rows = pool["disagree"]
elif mode == "balanced":
    half = n // 2
    rng.shuffle(pool["disagree"]); rng.shuffle(pool["agree"])
    rows = pool["disagree"][:half] + pool["agree"][:half]
else:
    rows = pool["all"]
rng.shuffle(rows)
dst.mkdir(parents=True, exist_ok=True)
with open(dst / "train.jsonl", "w") as f:
    f.writelines(rows[:n])
shutil.copy(src / "valid.jsonl", dst / "valid.jsonl")
shutil.copy(src / "valid.meta.jsonl", dst / "valid.meta.jsonl")
print(f"{min(n, len(rows))} of {len(rows)} rows ({mode or 'all'}) from {sorted(keep)} -> {dst}")
