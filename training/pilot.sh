#!/usr/bin/env bash
# One pilot: data from a generated-questions file only -> a short fine-tune -> the eval banks, both
# as written and typed casually. The same protocol as the first pilot (about 1 250 items, one epoch,
# CPU), so the numbers compare.
#   training/pilot.sh NAME GEN_JSONL [CASUAL]    (XDG_* at a clean `pistasho update`; PY; OUT with
#                                                  eval-cands.json and eval-cands-casual.json)
set -uo pipefail
cd "$(dirname "$0")/.."
name="$1"; gen="$2"; casual="${3:-0.8}"
OUT="${OUT:-training/out}"; PY="${PY:-python3}"
run="$OUT/pilot-$name"; mkdir -p "$run"
"$PY" training/make_data.py --out "$run/data" --generated "$gen" --generated-only --fmt td --casual "$casual" --workers 6 2>&1 | tail -2
"$PY" -m laya.train_cli --data "$run/data/train.jsonl" --eval "$run/data/valid.jsonl" --base multilingual \
  --out "$run/ckpt" --epochs 1 --micro-batch 4 --grad-accum 4 --head-max-len 512 --max-len 1024 \
  --loss soft-ce --shuffle-options --device cpu 2>&1 | tail -14
for bank in "" "-casual"; do
  echo "######## eval banks${bank:- as written}"
  "$PY" training/evaluate.py --model "$run/ckpt" --cache "$OUT/eval-cands$bank.json" --fmt td --cands 20 --device cpu \
    --json "$run/eval$bank.json" 2>&1 | grep -v -i "warn\|Fetching"
done
