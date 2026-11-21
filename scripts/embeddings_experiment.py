#!/usr/bin/env python3
"""Do multilingual embeddings improve `pistasho eval`? Measured before building anything (task 2.9a).

For each candidate model it embeds every tldr / cheat entry of the database, then answers the
questions of eval/questions.toml three ways and scores them like `pistasho eval` does (top-1 /
top-3 by tool):

  bm25    what `pistasho ask` returns today
  embed   cosine similarity only
  hybrid  both rankings fused with reciprocal rank fusion (k = 60)

It also reports what it costs: indexing time, per-question latency, model size on disk and memory.

Needs a venv with `fastembed` and `numpy` (not a dependency of the project, only of this study):

    python3 -m venv /tmp/emb && /tmp/emb/bin/pip install fastembed
    /tmp/emb/bin/python scripts/embeddings_experiment.py [model ...]

Models: e5-small-q8, minilm-l12, potion-128m. Embeddings are cached in /tmp so the fusion can be
re-run quickly. Uses the installed `pistasho` (BM25 side) and the real database, read-only.
"""
import json
import os
import re
import resource
import sqlite3
import subprocess
import sys
import time
import tomllib
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parent.parent
DB = Path(os.environ.get("PISTASHO_DB", Path.home() / ".local/share/pistasho/pistasho.db"))
BIN = os.environ.get("PISTASHO_BIN", str(Path.home() / ".cargo/bin/pistasho"))
CACHE = Path("/tmp")
RRF_K = 60
POOL = 60  # candidates kept from each ranking, as `ask` does

# name -> (fastembed model, query prefix, passage prefix)
MODELS = {
    "e5-small-q8": ("intfloat/multilingual-e5-small", "query: ", "passage: "),
    "minilm-l12": ("sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2", "", ""),
    "potion-128m": ("minishlab/potion-multilingual-128M", "", ""),
}


_LOADED = {}


def load_model(name):
    """Loaded once: fastembed refuses to register a custom model twice."""
    if name not in _LOADED:
        _LOADED[name] = _load(name)
    return _LOADED[name]


def _load(name):
    from fastembed import TextEmbedding
    from fastembed.common.model_description import ModelSource, PoolingType

    hf, _, _ = MODELS[name]
    if name == "e5-small-q8":
        # not in fastembed's catalogue: the int8 ONNX export of the same model
        TextEmbedding.add_custom_model(
            model=hf,
            pooling=PoolingType.MEAN,
            normalization=True,
            sources=ModelSource(hf="Xenova/multilingual-e5-small"),
            dim=384,
            model_file="onnx/model_quantized.onnx",
            size_in_gb=0.12,
        )
    return TextEmbedding(model_name=hf)


def entries():
    """tldr and cheat rows, the same ones `ask` searches (history is private and left out)."""
    con = sqlite3.connect(f"file:{DB}?mode=ro", uri=True)
    rows = con.execute(
        "select source, cmd, tool, description, alt from entries where source in ('tldr','cheat') order by id"
    ).fetchall()
    con.close()
    return rows


def passage(row):
    _, _, tool, desc, alt = row
    return f"{tool}. {desc}. {alt[:150]}"


def shape(cmd):
    """The command with its placeholder names erased: the same example in two languages is one."""
    return re.sub(r"\{\{.*?\}\}", "{{}}", cmd)


def embed_all(name, rows):
    cache = CACHE / f"emb-{name}.npy"
    if cache.exists():
        return np.load(cache), None
    model = load_model(name)
    _, _, pre = MODELS[name]
    texts = [pre + passage(r) for r in rows]
    t0 = time.time()
    vectors = np.array(list(model.embed(texts, batch_size=128)), dtype=np.float32)
    took = time.time() - t0
    np.save(cache, vectors)
    return vectors, took


def bm25(question):
    out = subprocess.run(
        [BIN, "ask", "--json", "-n", str(POOL), question],
        capture_output=True, text=True, env=dict(os.environ, HISTFILE="/dev/null"),
    )
    cands = json.loads(out.stdout)["candidates"] if out.stdout.strip() else []
    return [(c["source"], c["command"]) for c in cands]


def rrf(*rankings):
    score = {}
    for ranking in rankings:
        for pos, key in enumerate(ranking):
            score[key] = score.get(key, 0.0) + 1.0 / (RRF_K + pos + 1)
    return [k for k, _ in sorted(score.items(), key=lambda kv: -kv[1])]


def dedupe(keys, limit=POOL):
    seen, out = set(), []
    for key in keys:
        s = shape(key[1])
        if s not in seen:
            seen.add(s)
            out.append(key)
        if len(out) == limit:
            break
    return out


def score(ranked, tools_of, wanted):
    tools = [tools_of[k] for k in ranked[:3]]
    return int(bool(tools) and tools[0] in wanted), int(any(t in wanted for t in tools))


def run(name, rows, questions):
    vectors, index_secs = embed_all(name, rows)
    keys = [(r[0], r[1]) for r in rows]
    tools_of = {(r[0], r[1]): r[2] for r in rows}
    model = load_model(name)
    _, qpre, _ = MODELS[name]

    n = len(questions)
    res = {"bm25": [0, 0], "embed": [0, 0], "hybrid": [0, 0]}
    latencies = []
    bm25_cache = CACHE / "bm25-rankings.json"
    bm = json.loads(bm25_cache.read_text()) if bm25_cache.exists() else {}
    for q in questions:
        text = q["question"]
        if text not in bm:
            bm[text] = bm25(text)
        wanted = set(q["tools"])
        b = [tuple(k) for k in bm[text]]

        t0 = time.time()
        qv = np.array(next(iter(model.embed([qpre + text]))), dtype=np.float32)
        latencies.append(time.time() - t0)
        order = np.argsort(-(vectors @ qv))[:400]
        e = dedupe([keys[i] for i in order])

        for label, ranked in (("bm25", b), ("embed", e), ("hybrid", rrf(b, e))):
            t1, t3 = score(ranked, tools_of, wanted)
            res[label][0] += t1
            res[label][1] += t3
    bm25_cache.write_text(json.dumps(bm))

    size_mb = sum(f.stat().st_size for f in Path("/tmp/fastembed_cache").rglob("*") if f.is_file() and name_in_path(name, f)) / 1e6
    print(f"\n== {name}")
    print(f"   {len(rows)} entries, dim {vectors.shape[1]}, vectors {vectors.nbytes / 1e6:.0f} MB in memory")
    if index_secs:
        print(f"   indexing: {index_secs:.0f} s ({index_secs / len(rows) * 1000:.1f} ms per entry)")
    print(f"   one question: median {np.median(latencies) * 1000:.0f} ms, p95 {np.percentile(latencies, 95) * 1000:.0f} ms (warm)")
    print(f"   model on disk: {size_mb:.0f} MB, peak RSS of this process: {resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 1024:.0f} MB")
    for label in ("bm25", "embed", "hybrid"):
        t1, t3 = res[label]
        print(f"   {label:7} top-1 {t1 / n * 100:4.0f}%  top-3 {t3 / n * 100:4.0f}%")
    return res


def name_in_path(name, f):
    key = {"e5-small-q8": "multilingual-e5-small", "minilm-l12": "MiniLM-L12", "potion-128m": "potion"}[name]
    return key.lower() in str(f).lower()


def main():
    os.environ.setdefault("FASTEMBED_CACHE_PATH", "/tmp/fastembed_cache")
    wanted = sys.argv[1:] or list(MODELS)
    questions = tomllib.loads((ROOT / "eval/questions.toml").read_text())["q"]
    rows = entries()
    print(f"{len(questions)} questions, {len(rows)} entries")
    for name in wanted:
        run(name, rows, questions)


if __name__ == "__main__":
    main()
