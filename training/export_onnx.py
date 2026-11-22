#!/usr/bin/env python3
"""Exports a Laya checkpoint to a small ONNX model for CPU, and checks what was lost on the way.

    python3 training/export_onnx.py --ckpt out/laya-pistasho --out out/onnx --cache eval-cands.json \
        [--prune-from out/data/train.jsonl] [--no-int8]

Steps, each verified against the PyTorch model on a few eval items (max difference in the
probabilities):
  1. optional vocabulary pruning: the multilingual checkpoint carries 256 000 token embeddings
     (197M of its 307M parameters) and pistasho uses English and Spanish. Keeping only the tokens
     that occur in the given corpora (plus the special ones) makes the model several times smaller.
     The ids are remapped with `vocab_map.npy`; a token outside the kept set becomes <unk>.
  2. ONNX export (opset 17, dynamic lengths).
  3. dynamic int8 quantisation of the matrices (and the embedding table).
It prints the size on disk, the peak RSS and the latency of one 21-option decision.
"""
import argparse
import json
import os
import resource
import shutil
import statistics
import time
from pathlib import Path

import numpy as np
import torch

from common import FORMATS, build_row


def rss_mb():
    return resource.getrusage(resource.RUSAGE_SELF).ru_maxrss // 1024


def used_token_ids(tok, texts):
    ids = set(tok.all_special_ids)
    for i in range(0, len(texts), 512):
        for seq in tok(texts[i : i + 512], add_special_tokens=False)["input_ids"]:
            ids.update(seq)
    return ids


def corpus_texts(prune_from, cache):
    """Every text the model may read: the training rows and the eval questions with their candidates."""
    texts = []
    for path in prune_from:
        for line in open(path):
            row = json.loads(line)
            texts.append(row["state"])
            q = row["questions"]["pick"]
            texts.append(q["instructions"])
            texts.extend(q["criteria"].values())
    if cache:
        for items in json.load(open(cache)).values():
            for it in items:
                row = build_row(it["question"], it["cands"], "tdc")
                texts.append(row["state"])
                texts.extend(row["questions"]["pick"]["criteria"].values())
    return list(dict.fromkeys(texts))


def prune_vocab(agent, texts, out_dir, keep_first):
    """Slices the embedding table down to the tokens in `texts`; returns the old -> new id map."""
    emb = agent.model.encoder.embeddings.tok_embeddings
    old = emb.weight.shape[0]
    keep = used_token_ids(agent.tok, texts) | set(range(min(keep_first, old)))
    keep = sorted(i for i in keep if i < old)
    unk = agent.tok.unk_token_id if agent.tok.unk_token_id is not None else agent.tok.pad_token_id
    id_map = np.full(old, keep.index(unk) if unk in keep else 0, dtype=np.int64)
    for new, o in enumerate(keep):
        id_map[o] = new
    new_emb = torch.nn.Embedding(len(keep), emb.weight.shape[1], padding_idx=None)
    new_emb.weight.data = emb.weight.data[torch.tensor(keep)].clone()
    agent.model.encoder.embeddings.tok_embeddings = new_emb
    agent.model.encoder.config.vocab_size = len(keep)
    np.save(Path(out_dir) / "vocab_map.npy", id_map)
    print(f"vocabulary: {old} -> {len(keep)} tokens ({len(keep) / old * 100:.1f}%)")
    return id_map


class RemapSession:
    """Wraps an onnxruntime session so it can read ids of the original vocabulary."""

    def __init__(self, session, id_map):
        self.session, self.id_map = session, id_map

    def run(self, outputs, inputs):
        inputs = dict(inputs)
        inputs["input_ids"] = self.id_map[inputs["input_ids"]]
        return self.session.run(outputs, inputs)

    def __getattr__(self, name):
        return getattr(self.session, name)


def export(agent, path, ids_len=256):
    model = agent.model.eval().cpu().float()
    n = 4
    dummy = (
        torch.randint(5, 1000, (2, ids_len)),
        torch.ones(2, ids_len, dtype=torch.long),
        torch.tensor([[10, 50, 90, 130]] * 2),
        torch.ones(2, n, dtype=torch.bool),
        torch.zeros(2, dtype=torch.long),
    )
    torch.onnx.export(
        model, dummy, str(path),
        input_names=["input_ids", "attention_mask", "marker_pos", "marker_mask", "qtype"],
        output_names=["logits", "act_logits"],
        dynamic_axes={"input_ids": {0: "n", 1: "len"}, "attention_mask": {0: "n", 1: "len"},
                      "marker_pos": {0: "n", 1: "k"}, "marker_mask": {0: "n", 1: "k"}, "qtype": {0: "n"},
                      "logits": {0: "n", 1: "k"}, "act_logits": {0: "n"}},
        opset_version=17, dynamo=False,
    )


def quantize(src, dst, ops=("MatMul", "Gemm", "Gather"), per_channel=False, exclude=()):
    """Dynamic int8. What survives it is checked by `main`; `ops`, `per_channel` and `exclude` are the knobs."""
    import onnx
    from onnxruntime.quantization import QuantType, quantize_dynamic

    nodes = [n.name for n in onnx.load(str(src), load_external_data=False).graph.node
             if any(x in n.name for x in exclude)]
    quantize_dynamic(str(src), str(dst), weight_type=QuantType.QInt8, op_types_to_quantize=list(ops),
                     per_channel=per_channel, nodes_to_exclude=nodes)


def probs_of(agent_like, row):
    res = agent_like.predict(row["state"], row["questions"])
    return res["answers"]["pick"]["probabilities"]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--ckpt", required=True, help="checkpoint directory (or hub id) to export")
    ap.add_argument("--subfolder", default=None)
    ap.add_argument("--out", required=True)
    ap.add_argument("--cache", required=True, help="eval-cands.json: items to verify on")
    ap.add_argument("--prune-from", nargs="*", default=[], help="JSONL files whose tokens are kept (enables pruning)")
    ap.add_argument("--keep-first", type=int, default=4000, help="always keep the first N token ids")
    ap.add_argument("--no-int8", action="store_true")
    ap.add_argument("--verify", type=int, default=12)
    ap.add_argument("--cands", type=int, default=20)
    ap.add_argument("--fmt", choices=FORMATS, default="td")
    args = ap.parse_args()

    import laya
    from onnxruntime import InferenceSession

    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    kw = {"subfolder": args.subfolder} if args.subfolder else {}
    agent = laya.load(args.ckpt, device="cpu", **kw)
    items = [it for items in json.load(open(args.cache)).values() for it in items if it["cands"]]
    sample = items[:: max(1, len(items) // args.verify)][: args.verify]
    rows = [build_row(it["question"], it["cands"], args.fmt, args.cands) for it in sample]
    reference = [probs_of(agent, r) for r in rows]

    id_map = None
    if args.prune_from:
        id_map = prune_vocab(agent, corpus_texts(args.prune_from, args.cache), out, args.keep_first)

    # the runtime reads the config and the tokenizer next to the graph, like any Laya checkpoint
    src = Path(args.ckpt)
    if not src.is_dir():  # a hub id: its files are already in the Hugging Face cache after laya.load
        from huggingface_hub import snapshot_download

        patterns = [f"{args.subfolder + '/' if args.subfolder else ''}rl_agent_config.json"]
        src = Path(snapshot_download(args.ckpt, allow_patterns=patterns))
    if args.subfolder:
        src = src / args.subfolder
    shutil.copy(src / "rl_agent_config.json", out / "rl_agent_config.json")
    tok_dst = out / "tokenizer"
    agent.tok.save_pretrained(tok_dst)

    t = time.time()
    fp32 = out / "laya.onnx"
    export(agent, fp32)
    print(f"exported in {time.time() - t:.0f} s: {fp32.stat().st_size / 1e6:.0f} MB (+ external data: "
          f"{sum(f.stat().st_size for f in out.glob('*.data')) / 1e6:.0f} MB)")
    result = {"fp32": fp32}
    if not args.no_int8:
        int8 = out / "laya.int8.onnx"
        quantize(fp32, int8)
        print(f"int8: {int8.stat().st_size / 1e6:.0f} MB")
        result["int8"] = int8

    for label, path in result.items():
        runner = laya.load(str(out), backend="onnx", onnx_path=str(path))
        if id_map is not None:
            runner.session = RemapSession(runner.session, id_map)
        diffs, secs = [], []
        for row, ref in zip(rows, reference):
            t = time.time()
            got = probs_of(runner, row)
            secs.append(time.time() - t)
            diffs.append(max(abs(got.get(k, 0.0) - v) for k, v in ref.items()))
        print(f"{label}: max |Δp| vs PyTorch {max(diffs):.3f} (mean {statistics.mean(diffs):.3f}), "
              f"median {statistics.median(secs) * 1000:.0f} ms, peak RSS {rss_mb()} MB")
        del runner


if __name__ == "__main__":
    main()
