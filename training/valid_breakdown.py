#!/usr/bin/env python3
"""Validation accuracy broken down by where the question came from, next to the local order.

Needs the `valid.meta.jsonl` that make_data.py writes beside `valid.jsonl` (same row order). For every
held-out row (tools the model never saw in training) it reports, per question source:

  * local   : the local order's first candidate was a right tool (before the right one was injected)
  * model   : the checkpoint's pick is a right tool (tool-level, like `pistasho eval`)
  * exact   : the pick is the gold argmax option (what laya-train reports as Accuracy; "none" counts)
  * none    : how often the model says "none", and the gold share of "none"

    python3 training/valid_breakdown.py --data training/out/data-meta --model training/out/runs/ladder-40000/ckpt
    python3 training/valid_breakdown.py --data training/out/data-meta            # local order only
"""
import argparse
import json
from collections import defaultdict


def tools_of(criteria):
    return {k: v.split(": ", 1)[0] for k, v in criteria.items() if k != "none"}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--data", required=True, help="folder with valid.jsonl and valid.meta.jsonl")
    ap.add_argument("--model", default=None, help="checkpoint; without it only the local order is shown")
    ap.add_argument("--device", default=None)
    ap.add_argument("--limit", type=int, default=0)
    ap.add_argument("--json", default=None)
    args = ap.parse_args()

    rows = [json.loads(l) for l in open(f"{args.data}/valid.jsonl")]
    metas = [json.loads(l) for l in open(f"{args.data}/valid.meta.jsonl")]
    assert len(rows) == len(metas), "valid.jsonl and valid.meta.jsonl differ in length"
    if args.limit:
        rows, metas = rows[: args.limit], metas[: args.limit]
    model = None
    if args.model:
        import laya

        model = laya.load(args.model, device=args.device)

    acc = defaultdict(lambda: defaultdict(int))
    for row, meta in zip(rows, metas):
        pick_q = row["questions"]["pick"]["criteria"]
        tools = tools_of(pick_q)
        probs = row["gold"]["pick"]["probabilities"]
        gold_none = probs.get("none", 0) >= 1.0
        gold_ids = {k for k, v in probs.items() if v > 0 and k != "none"}
        gold_tools = {tools[k] for k in gold_ids}
        best = max(probs, key=probs.get)
        choice = None
        if model:
            choice = model.predict(row["state"], row["questions"])["answers"]["pick"]["choice"]
        for key in (meta["src"], "ALL", "ALL none" if gold_none else "ALL non-none"):
            a = acc[key]
            a["n"] += 1
            a["gold_none"] += gold_none
            a["local"] += bool(meta["loc_tool"]) and not gold_none
            if model:
                a["says_none"] += choice == "none"
                a["exact"] += choice == best
                a["model"] += (choice == "none") if gold_none else (choice != "none" and tools[choice] in gold_tools)
    print_table(acc, model is not None)
    if args.json:
        json.dump({k: dict(v) for k, v in acc.items()}, open(args.json, "w"), indent=1)


def print_table(acc, with_model):
    keys = sorted(k for k in acc if not k.startswith("ALL")) + ["ALL non-none", "ALL none", "ALL"]
    head = f"{'source':16s} {'rows':>6s} {'gold-none':>9s} {'local':>7s}"
    if with_model:
        head += f" {'model':>7s} {'exact':>7s} {'says-none':>9s}"
    print(head)
    for k in keys:
        a = acc[k]
        n = a["n"]
        nn = n - a["gold_none"]
        line = f"{k:16s} {n:6d} {a['gold_none'] / n:9.0%} {a['local'] / max(nn, 1):7.0%}"
        if with_model:
            line += f" {a['model'] / n:7.0%} {a['exact'] / n:7.0%} {a['says_none'] / n:9.0%}"
        print(line)
    print("local = first local candidate right, over rows whose gold is not 'none'; model = tool-level over ALL rows (none rows right if it says none)")


if __name__ == "__main__":
    main()
