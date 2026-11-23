#!/usr/bin/env python3
"""The closing table of the 5090 session: every run with its cost, top-1 / top-3 on the four banks, 'none', and the fusion
with the local order (weights picked on one fixed half of a bank and measured on the other), plus the seed groups.

    python3 training/final_table.py training/out > training/out/FINAL_TABLE.md
"""
import json
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from fuse import LAMS, WEIGHTS, metrics  # noqa: E402

out = Path(sys.argv[1] if len(sys.argv) > 1 else "training/out")
runs = out / "runs"
BANKS = (("orig", ""), ("synth", ""), ("orig", "-casual"), ("synth", "-casual"))
GROUPS = {
    "rank40k (3 seeds)": ["rank40k", "rank40k-seed1", "rank40k-seed2"],
    "disagree (3 seeds)": ["dis-s0", "dis-s1", "dis-s2"],
}


def load(p):
    return json.load(open(p)) if p.exists() else None


def cost(run):
    log = (runs / run / "train.log")
    m = re.search(r"trained (\d+) rows x (\d+) epochs in (\d+) s", log.read_text()) if log.exists() else None
    return (int(m[1]), int(m[2]), int(m[3])) if m else (0, 0, 0)


def top(run):
    """{(bank, style): (top1, top3)} of the model and of the local order."""
    got, loc = {}, {}
    for bank, style in BANKS:
        r = load(runs / run / f"eval{style}.json")
        if r and bank in r:
            got[(bank, style)] = (r[bank]["model_top1"], r[bank]["model_top3"])
            loc[(bank, style)] = (r[bank]["local_top1"], r[bank]["local_top3"])
    return got, loc


def fused(run, style):
    """Split-half fusion top-1 (rrf, lin) per bank: weight chosen on one half, measured on the other, mean of the two."""
    d = load(runs / run / f"dump{style}.json")
    res = {}
    if not d:
        return res
    for bank, items in d.items():
        halves = [items[0::2], items[1::2]]
        for method, grid in (("rrf", WEIGHTS), ("lin", LAMS)):
            got = []
            for pick, test in ((0, 1), (1, 0)):
                best = max(grid, key=lambda w: (metrics(halves[pick], method, w)[0], -abs(w - 1)))
                got.append(metrics(halves[test], method, best)[0])
            res[(bank, method)] = sum(got) / 2
    return res


names = sorted(p.parent.name for p in runs.glob("*/eval.json"))
print("| run | filas | épocas | min | orig | synth | orig casual | synth casual | none | fusión rrf orig/synth | fusión lin orig/synth |")
print("|---|---|---|---|---|---|---|---|---|---|---|")
local_row = None
for n in names:
    got, loc = top(n)
    local_row = local_row or loc
    rows, ep, sec = cost(n)
    r = load(runs / n / "eval.json")
    none = (r["orig"]["none_rate"] + r["synth"]["none_rate"]) / 2 * 100
    f = fused(n, "")
    cells = [f"{got[k][0] * 100:.0f} / {got[k][1] * 100:.0f}" if k in got else "-" for k in BANKS]
    fr = f" {f[('orig', 'rrf')] * 100:.0f} / {f[('synth', 'rrf')] * 100:.0f}" if ("orig", "rrf") in f else " -"
    fl = f" {f[('orig', 'lin')] * 100:.0f} / {f[('synth', 'lin')] * 100:.0f}" if ("orig", "lin") in f else " -"
    print(f"| {n} | {rows} | {ep} | {sec / 60:.1f} | " + " | ".join(cells) + f" | {none:.0f} % |{fr} |{fl} |")
if local_row:
    cells = [f"{local_row[k][0] * 100:.0f} / {local_row[k][1] * 100:.0f}" for k in BANKS]
    print("| **orden local** | - | - | - | " + " | ".join(cells) + " | - | - | - |")
print("| **Jev (referencia)** | - | - | - | 83 / 92 | 76 / 88 | - | - | - | - | - |")

print("\n### Grupos de semillas: top-1 del modelo menos top-1 del orden local, en puntos (mín … máx entre semillas)\n")
print("| grupo | orig | synth | orig casual | synth casual |")
print("|---|---|---|---|---|")
for g, members in GROUPS.items():
    members = [m for m in members if (runs / m / "eval.json").exists()]
    if not members:
        continue
    cells = []
    for k in BANKS:
        diffs = []
        for m in members:
            got, loc = top(m)
            if k in got:
                diffs.append((got[k][0] - loc[k][0]) * 100)
        cells.append(f"{min(diffs):+.0f} … {max(diffs):+.0f}" if diffs else "-")
    print(f"| {g}: {', '.join(members)} | " + " | ".join(cells) + " |")
