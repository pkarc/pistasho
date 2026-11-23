#!/usr/bin/env python3
"""Does the model add something to the local order, or only replace it? (works on `evaluate.py --dump` files)

Fuses the model's probability for each candidate with the local position/score, which is what pistasho
would do at run time (the model re-reads the options but does not see where the search put them):

  rrf    score = 1/(k + local rank) + w / (k + model rank)        (k = 10, as ask.rs)
  lin    score = p_model + lam * local_score                      (local_score is ask's fused 0..1 score)

Reports top-1 / top-3 per bank for a sweep of the weight, the weight picked on one bank applied to the
other (so the sweep does not grade itself), and the keeps/breaks table of the model alone versus the local order.

    python3 training/fuse.py training/out/runs/ladder-40000/dump.json
"""
import json
import sys

from common import right

K = 10
WEIGHTS = [0, 0.25, 0.5, 0.75, 1, 1.5, 2, 3, 5, 100]   # w = 0 is the local order, 100 is the model alone
LAMS = [0, 0.1, 0.25, 0.5, 1, 2, 4, 100]


def ranks(vals):
    order = sorted(range(len(vals)), key=lambda i: -vals[i])
    r = [0] * len(vals)
    for pos, i in enumerate(order):
        r[i] = pos
    return r


def order_of(item, method, w):
    n = len(item["p"])
    if method == "rrf":
        rm = ranks(item["p"])
        score = [1 / (K + i) + w / (K + rm[i]) for i in range(n)]
    else:
        score = [item["p"][i] + w * item["local_score"][i] for i in range(n)]
    return sorted(range(n), key=lambda i: (-score[i], i))


def metrics(items, method, w):
    t1 = t3 = 0
    for it in items:
        o = order_of(it, method, w)
        ok = [right(it["cand_tools"][i], it["tools"]) for i in range(len(o))]
        t1 += ok[o[0]]
        t3 += any(ok[i] for i in o[:3])
    return t1 / len(items), t3 / len(items)


def main():
    path = sys.argv[1]
    banks = json.load(open(path))
    for method, grid in (("rrf", WEIGHTS), ("lin", LAMS)):
        print(f"\n--- {method}: weight of the model (rrf: w, 0 = local, 100 = model alone) / lam on local score (lin: 0 = model alone, 100 = local alone)")
        print(f"{'w':>6s}  " + "  ".join(f"{b:>11s}" for b in banks))
        curves = {b: {} for b in banks}
        for w in grid:
            row = []
            for b, items in banks.items():
                t1, t3 = metrics(items, method, w)
                curves[b][w] = t1
                row.append(f"{t1:5.0%} / {t3:3.0%}")
            print(f"{w:6}  " + "  ".join(f"{r:>11s}" for r in row))
        names = list(banks)
        if len(names) == 2:
            for src, dst in ((names[0], names[1]), (names[1], names[0])):
                best = max(grid, key=lambda w: (curves[src][w], -abs(w - 1)))
                print(f"picked on {src}: w={best} -> {dst} top-1 {curves[dst][best]:.0%} (w=0/alone: {curves[dst][grid[0]]:.0%} / {curves[dst][grid[-1]]:.0%})")
    print("\n--- split halves: weight picked on one fixed half of a bank (even / odd questions), measured on the other; mean of the two held-out halves")
    for method, grid in (("rrf", WEIGHTS), ("lin", LAMS)):
        for b, items in banks.items():
            halves = [items[0::2], items[1::2]]
            got, local, alone = [], [], []
            for pick, test in ((0, 1), (1, 0)):
                best = max(grid, key=lambda w: (metrics(halves[pick], method, w)[0], -abs(w - 1)))
                got.append(metrics(halves[test], method, best)[0])
                local.append(metrics(halves[test], method, 0 if method == "rrf" else 100)[0])
                alone.append(metrics(halves[test], method, 100 if method == "rrf" else 0)[0])
            print(f"{method} {b:6s}: fused {sum(got) / 2:4.0%} (halves {got[0]:.0%}, {got[1]:.0%}) | local {sum(local) / 2:4.0%} | model alone {sum(alone) / 2:4.0%}")
    print("\n--- model alone versus local order, top-1 (rows: local right/wrong, columns: model right/wrong)")
    for b, items in banks.items():
        c = {"keep": 0, "break": 0, "fix": 0, "both_wrong": 0}
        for it in items:
            ok = [right(t, it["tools"]) for t in it["cand_tools"]]
            loc = ok[0]
            mod = ok[max(range(len(it["p"])), key=lambda i: it["p"][i])]
            c["keep" if loc and mod else "break" if loc else "fix" if mod else "both_wrong"] += 1
        n = len(items)
        print(f"{b:6s} n={n}: local right & model right {c['keep']}, local right & model wrong (broken) {c['break']}, "
              f"local wrong & model right (fixed) {c['fix']}, both wrong {c['both_wrong']}  | oracle top-1 {(c['keep'] + c['break'] + c['fix']) / n:.0%}")


if __name__ == "__main__":
    main()
