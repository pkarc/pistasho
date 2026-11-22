#!/usr/bin/env python3
"""Freezes the local candidates of every eval question into one JSON file.

The training machine does not need pistasho or its database: `evaluate.py` reads this file.
Run it with the database the model will be used with (the XDG_* of a clean `pistasho update`).

    XDG_DATA_HOME=... XDG_CONFIG_HOME=... python3 training/make_eval_cache.py training/out/eval-cands.json [--casual]
"""
import json
import sys
from concurrent.futures import ThreadPoolExecutor

from common import eval_questions, local_candidates
from style import casual_text

out = sys.argv[1] if len(sys.argv) > 1 else "eval-cands.json"
banks = eval_questions()
if "--casual" in sys.argv:
    # the same questions typed the way people do (style.py): lowercase, no accents, no ¿ or final stop
    banks = {n: [{**q, "question": casual_text(q["question"])} for q in qs] for n, qs in banks.items()}
result = {}
with ThreadPoolExecutor(6) as pool:
    for name, qs in banks.items():
        cands = list(pool.map(lambda q: local_candidates(q["question"]), qs))
        result[name] = [{"question": q["question"], "tools": q["tools"], "cands": c} for q, c in zip(qs, cands)]
        print(f"{name}: {len(qs)} questions, {sum(1 for c in cands if c)} with candidates")
json.dump(result, open(out, "w"), ensure_ascii=False)
print("wrote", out)
