#!/usr/bin/env python3
"""A random sample of the training rows (same seed, same rows), keeping the validation file whole.

    python3 training/subsample.py data-td 10000 data-ladder-10000
"""
import random
import shutil
import sys
from pathlib import Path

src, n, dst = Path(sys.argv[1]), int(sys.argv[2]), Path(sys.argv[3])
rows = [l for l in open(src / "train.jsonl")]
random.Random(7).shuffle(rows)
dst.mkdir(parents=True, exist_ok=True)
with open(dst / "train.jsonl", "w") as f:
    f.writelines(rows[:n])
shutil.copy(src / "valid.jsonl", dst / "valid.jsonl")
print(f"{min(n, len(rows))} of {len(rows)} rows -> {dst}")
