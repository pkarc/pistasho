#!/usr/bin/env python3
"""Would a reranker on top of `pistasho ask` pay off? Measured before building anything (task 3a).

1. Recall@k of the current retrieval (BM25 + embeddings): the ceiling of any reranker.
2. For each cross-encoder: top-1 / top-3 after reranking the best N candidates, and the
   time it takes (CPU and GPU), on both question sets.

Needs a venv with torch, sentence-transformers and transformers (study only, not a project
dependency), the release binary and a database built by `pistasho update`:

    python3 -m venv /tmp/venv && /tmp/venv/bin/pip install torch sentence-transformers
    XDG_DATA_HOME=... /tmp/venv/bin/python scripts/rerank_experiment.py [model ...]

Models: bge-m3, horizon-small, jina-v2, mmarco.
"""
import json
import os
import subprocess
import sys
import time
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BIN = os.environ.get("PISTASHO_BIN", str(ROOT / "target/release/pistasho"))
CACHE = Path(os.environ.get("RERANK_CACHE", Path.home() / ".cache/pk-x"))
POOL = 30
SETS = {"orig": ROOT / "eval/questions.toml", "synth": ROOT / "eval/questions-synth.toml"}
MODELS = {
    "bge-m3": "BAAI/bge-reranker-v2-m3",
    "horizon-small": "Horizon-Labs/multilingual-reranker-small",
    "jina-v2": "jinaai/jina-reranker-v2-base-multilingual",
    "mmarco": "cross-encoder/mmarco-mMiniLMv2-L12-H384-v1",
}


def candidates(question):
    out = subprocess.run(
        [BIN, "ask", "--json", "-n", str(POOL), question],
        capture_output=True, text=True, env=dict(os.environ, HISTFILE="/dev/null"),
    )
    return json.loads(out.stdout)["candidates"] if out.stdout.strip() else []


def load_set(name):
    qs = tomllib.loads(SETS[name].read_text())["q"]
    cache = CACHE / f"cands-{name}.json"
    if cache.exists():
        return qs, json.loads(cache.read_text())
    cands = {q["question"]: candidates(q["question"]) for q in qs}
    cache.write_text(json.dumps(cands))
    return qs, cands


def passage(c):
    return f'{c["tool"]}: {c["description"]} | {c["command"]}'


def hit(cands, wanted, k):
    return any(c["tool"] in wanted for c in cands[:k])


def report(label, qs, ranked):
    n = len(qs)
    t1 = sum(hit(ranked[q["question"]], q["tools"], 1) for q in qs)
    t3 = sum(hit(ranked[q["question"]], q["tools"], 3) for q in qs)
    print(f"   {label:28} top-1 {t1 / n * 100:4.0f}%  top-3 {t3 / n * 100:4.0f}%")


def recall(qs, cands):
    n = len(qs)
    row = "   recall@k  " + "  ".join(
        f"{k}: {sum(hit(cands[q['question']], q['tools'], k) for q in qs) / n * 100:3.0f}%"
        for k in (1, 3, 5, 10, 20, 30)
    )
    print(row)


def run_model(key, data):
    import torch
    from sentence_transformers import CrossEncoder

    device = "cuda" if torch.cuda.is_available() else "cpu"
    kwargs = {"trust_remote_code": True} if key == "jina-v2" else {}
    for dev in dict.fromkeys(["cpu", device]):
        model = CrossEncoder(MODELS[key], device=dev, max_length=256, **kwargs)
        print(f"\n== {key} ({MODELS[key]}) on {dev}")
        for name, (qs, cands) in data.items():
            for pool in (10, 20):
                ranked, secs = {}, []
                for q in qs:
                    cs = cands[q["question"]][:pool]
                    if not cs:
                        ranked[q["question"]] = []
                        continue
                    t0 = time.time()
                    scores = model.predict([(q["question"], passage(c)) for c in cs], batch_size=32)
                    secs.append(time.time() - t0)
                    ranked[q["question"]] = [c for _, c in sorted(zip(scores, cs), key=lambda p: -p[0])]
                    # keep the order stable for ties: sorted is stable
                med = sorted(secs)[len(secs) // 2] * 1000 if secs else 0
                report(f"{name} rerank top-{pool}", qs, ranked)
                if name == "synth":
                    print(f"   {'':28} median {med:.0f} ms per question for {pool} candidates")
        del model


def main():
    data = {name: load_set(name) for name in SETS}
    for name, (qs, cands) in data.items():
        print(f"== retrieval, {name}: {len(qs)} questions")
        recall(qs, cands)
        report("current order", qs, cands)
    for key in sys.argv[1:] or MODELS:
        try:
            run_model(key, data)
        except Exception as e:  # a model that does not load must not stop the others
            print(f"\n== {key}: FAILED: {e}")


if __name__ == "__main__":
    main()
