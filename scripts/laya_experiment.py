#!/usr/bin/env python3
"""Does Laya (the open decision model, run locally) come close to Jev, and at what cost?

Runs the same `choice` experiment as scripts/jev_experiment.py (the best local candidates + "none",
same prompts) with the model on this machine instead of the hosted API, and reports the memory and
the time it needs. Study only: needs a venv with `torch` (CPU) and `laya`.

    python3 -m venv /tmp/venv && /tmp/venv/bin/pip install torch --index-url https://download.pytorch.org/whl/cpu
    /tmp/venv/bin/pip install laya numpy
    XDG_DATA_HOME=... JEV_CANDS=20 JEV_CACHE=/tmp/laya-results /tmp/venv/bin/python scripts/laya_experiment.py [multilingual|english]
"""
import importlib.util
import os
import resource
import sys
import time
from pathlib import Path

spec = importlib.util.spec_from_file_location("jev_experiment", Path(__file__).with_name("jev_experiment.py"))
j = importlib.util.module_from_spec(spec)
spec.loader.exec_module(j)

import laya  # noqa: E402

which = sys.argv[1] if len(sys.argv) > 1 else "multilingual"
t0 = time.time()
model = laya.load("convaiinnovations/laya", subfolder=which) if which == "multilingual" else laya.load("convaiinnovations/laya")
rss = lambda: resource.getrusage(resource.RUSAGE_SELF).ru_maxrss // 1024
print(f"laya {which}: loaded in {time.time() - t0:.1f} s, peak RSS {rss()} MB")


def call(state, questions, retries=0):
    t = time.time()
    out = model.predict(state, questions)
    out["_secs"] = time.time() - t
    return out


j.call = call
j.CACHE.mkdir(parents=True, exist_ok=True)
j.WORKERS = 1  # the model is the bottleneck, not the network
j.choice_experiment()
print(f"\npeak RSS after the run: {rss()} MB")
