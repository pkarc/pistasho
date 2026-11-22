#!/usr/bin/env python3
"""Scores a Laya checkpoint (or its ONNX export) on the pistasho eval banks.

Same measurements as scripts/jev_experiment.py, so the numbers sit next to Jev's and the local
order's (docs/eval.md): top-1 / top-3 after re-ordering the 20 local candidates, how often it says
"none" and whether that is right, how sure it is when it is right, and the time and memory it needs.

    python3 training/evaluate.py --model convaiinnovations/laya --subfolder multilingual --cache eval-cands.json
    python3 training/evaluate.py --model out/laya-pistasho --cache eval-cands.json
    python3 training/evaluate.py --model out/laya-pistasho --onnx out/onnx/laya.int8.onnx --cache eval-cands.json
"""
import argparse
import json
import resource
import statistics
import time

from common import FORMATS, build_row, right


def rss_mb():
    return resource.getrusage(resource.RUSAGE_SELF).ru_maxrss // 1024


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--model", required=True, help="checkpoint directory or hub id")
    ap.add_argument("--subfolder", default=None)
    ap.add_argument("--onnx", default=None, help="an exported .onnx file next to the checkpoint's config")
    ap.add_argument("--cache", required=True, help="eval-cands.json from make_eval_cache.py")
    ap.add_argument("--device", default=None)
    ap.add_argument("--limit", type=int, default=0, help="only the first N questions of each bank")
    ap.add_argument("--cands", type=int, default=20, help="how many local candidates are put to the model")
    ap.add_argument("--fmt", choices=FORMATS, default="td")
    ap.add_argument("--vocab-map", default=None, help="vocab_map.npy of a pruned export (with --onnx)")
    ap.add_argument("--json", default=None, help="write the numbers here too")
    args = ap.parse_args()

    import laya

    t0 = time.time()
    kw = {"subfolder": args.subfolder} if args.subfolder else {}
    if args.onnx:
        model = laya.load(args.model, backend="onnx", onnx_path=args.onnx, **kw)
        if args.vocab_map:
            from export_onnx import RemapSession
            import numpy as np

            model.session = RemapSession(model.session, np.load(args.vocab_map))
    else:
        model = laya.load(args.model, device=args.device, **kw)
    print(f"loaded in {time.time() - t0:.1f} s, peak RSS {rss_mb()} MB")

    banks = json.load(open(args.cache))
    summary = {}
    for name, items in banks.items():
        items = [{**i, "cands": i["cands"][: args.cands]} for i in items if i["cands"]]
        if args.limit:
            items = items[: args.limit]
        rows, secs = [], []
        for it in items:
            row = build_row(it["question"], it["cands"], args.fmt)
            t = time.time()
            res = model.predict(row["state"], row["questions"])
            secs.append(time.time() - t)
            pick = res["answers"]["pick"]
            probs = pick["probabilities"]
            n = len(it["cands"])
            order = sorted(range(n), key=lambda i: -probs.get(f"c{i + 1}", 0.0))
            rows.append({"it": it, "order": order, "none": pick["choice"] == "none", "pick": pick["choice"],
                         "conf": pick.get("answer_confidence", pick.get("confidence", 0.0)),
                         "none_p": probs.get("none", 0.0)})
        d = len(rows)
        ok = lambda r, i: right(r["it"]["cands"][i]["tool"], r["it"]["tools"])
        loc1 = sum(ok(r, 0) for r in rows)
        loc3 = sum(any(ok(r, i) for i in range(min(3, len(r["it"]["cands"])))) for r in rows)
        rec = sum(any(ok(r, i) for i in range(len(r["it"]["cands"]))) for r in rows)
        m1 = sum(ok(r, r["order"][0]) for r in rows)
        m3 = sum(any(ok(r, i) for i in r["order"][:3]) for r in rows)
        none = [r for r in rows if r["none"]]
        some = [r for r in rows if not r["none"]]
        loc_in_none = sum(ok(r, 0) for r in none) / max(1, len(none))
        loc_in_some = sum(ok(r, 0) for r in some) / max(1, len(some))
        # when it names a candidate: how often right, by how sure it is
        buckets = {}
        for r in some:
            idx = int(r["pick"][1:]) - 1
            b = "≥0.95" if r["conf"] >= 0.95 else "0.8–0.95" if r["conf"] >= 0.8 else "<0.8"
            a, t = buckets.get(b, (0, 0))
            buckets[b] = (a + ok(r, idx), t + 1)
        secs.sort()
        print(f"\n== {name}: {d} questions")
        print(f"   local order : top-1 {loc1 / d * 100:3.0f}%  top-3 {loc3 / d * 100:3.0f}%   (recall@{args.cands} {rec / d * 100:.0f}%)")
        print(f"   model       : top-1 {m1 / d * 100:3.0f}%  top-3 {m3 / d * 100:3.0f}%")
        print(f"   'none'      : {len(none)} ({len(none) / d * 100:.0f}%); local top-1 right in {loc_in_none * 100:.0f}% of them vs {loc_in_some * 100:.0f}% otherwise")
        print("   right by confidence: " + ", ".join(f"{b} {a / t * 100:.0f}% of {t}" for b, (a, t) in sorted(buckets.items())))
        print(f"   latency     : median {statistics.median(secs) * 1000:.0f} ms, p95 {secs[int(len(secs) * .95)] * 1000:.0f} ms")
        summary[name] = {"n": d, "local_top1": loc1 / d, "local_top3": loc3 / d, "model_top1": m1 / d,
                         "model_top3": m3 / d, "none_rate": len(none) / d, "recall": rec / d,
                         "median_ms": statistics.median(secs) * 1000}
    summary["peak_rss_mb"] = rss_mb()
    print(f"\npeak RSS {rss_mb()} MB")
    if args.json:
        json.dump(summary, open(args.json, "w"), indent=1)


if __name__ == "__main__":
    main()
