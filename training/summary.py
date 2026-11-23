#!/usr/bin/env python3
"""One table with every trained variant next to the references (local order, Jev, zero-shot).

Each variant is shown on the eval banks as written and typed casually (style.py), top-1 / top-3.
"""
import json
import sys
from pathlib import Path

out = Path(sys.argv[1] if len(sys.argv) > 1 else "training/out")
JEV = {"orig": (83, 92), "synth": (76, 88)}  # docs/eval.md, 20 candidates


def load(path):
    return json.load(open(path)) if path.exists() else None


def cell(r, bank, key="model"):
    if not r or bank not in r:
        return "      -"
    return f"{r[bank][key + '_top1'] * 100:>3.0f} /{r[bank][key + '_top3'] * 100:>3.0f}"


rows = []
base = load(out / "baseline.json")
if base:
    rows.append(("zero-shot Laya", base, load(out / "baseline-casual.json")))
for p in sorted((out / "runs").glob("*/eval.json")):
    rows.append((p.parent.name, load(p), load(p.parent / "eval-casual.json")))
print(f"{'variant':18} {'real':>9} {'synth':>9} | {'real casual':>11} {'synth casual':>12} | {'none':>5} {'ms':>5} {'RSS MB':>7}")
if rows:
    f, fc = rows[0][1], rows[0][2]
    print(f"{'local order':18} {cell(f, 'orig', 'local'):>9} {cell(f, 'synth', 'local'):>9} | {cell(fc, 'orig', 'local'):>11} {cell(fc, 'synth', 'local'):>12} |")
print(f"{'Jev (reference)':18} {JEV['orig'][0]:>5} /{JEV['orig'][1]:>3} {JEV['synth'][0]:>5} /{JEV['synth'][1]:>3} |")
for name, r, rc in rows:
    none = (r["orig"]["none_rate"] + r["synth"]["none_rate"]) / 2 * 100
    ms = max(r["orig"]["median_ms"], r["synth"]["median_ms"])
    print(f"{name:18} {cell(r, 'orig'):>9} {cell(r, 'synth'):>9} | {cell(rc, 'orig'):>11} {cell(rc, 'synth'):>12} | {none:>4.0f}% {ms:>5.0f} {r['peak_rss_mb']:>7}")
