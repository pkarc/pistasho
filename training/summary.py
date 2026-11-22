#!/usr/bin/env python3
"""One table with every trained variant next to the references (local order, Jev, zero-shot)."""
import json
import sys
from pathlib import Path

out = Path(sys.argv[1] if len(sys.argv) > 1 else "training/out")
JEV = {"orig": (83, 92), "synth": (76, 88)}  # docs/eval.md, 20 candidates
rows = []
base = out / "baseline.json"
if base.exists():
    b = json.load(open(base))
    rows.append(("zero-shot Laya", b, None))
for p in sorted((out / "runs").glob("*/eval.json")):
    rows.append((p.parent.name, json.load(open(p)), p.parent))
print(f"{'variant':16} {'orig top-1/3':>14} {'synth top-1/3':>14} {'none':>6} {'ms':>6} {'RSS MB':>7}")
first = rows[0][1] if rows else None
if first:
    print(f"{'local order':16} {first['orig']['local_top1']*100:>8.0f} /{first['orig']['local_top3']*100:>3.0f} "
          f"{first['synth']['local_top1']*100:>9.0f} /{first['synth']['local_top3']*100:>3.0f}")
print(f"{'Jev (reference)':16} {JEV['orig'][0]:>8} /{JEV['orig'][1]:>3} {JEV['synth'][0]:>9} /{JEV['synth'][1]:>3}")
for name, r, _ in rows:
    o, s = r["orig"], r["synth"]
    print(f"{name:16} {o['model_top1']*100:>8.0f} /{o['model_top3']*100:>3.0f} {s['model_top1']*100:>9.0f} /{s['model_top3']*100:>3.0f} "
          f"{(o['none_rate'] + s['none_rate']) / 2 * 100:>5.0f}% {max(o['median_ms'], s['median_ms']):>6.0f} {r['peak_rss_mb']:>7}")
