#!/usr/bin/env bash
# Experiments queued behind the 100k ladder step: more epochs on the same 40k, and a Gemma+NL2Bash-only 40k.
set -uo pipefail
cd "$(dirname "$0")/.."
while pgrep -f "run.sh ladder" >/dev/null; do sleep 10; done
PY=training/out/venv/bin/python
$PY training/subset_by_source.py training/out/data-meta 40000 training/out/data-gemma40k gen-gemma gen-gemma-b nl2bash
cp training/out/data-ladder-40000/train.jsonl /dev/null
EPOCHS=3 training/run.sh train e3-mix40k td 20 512 training/out/data-ladder-40000
EPOCHS=1 training/run.sh train gemma40k td 20 512 training/out/data-gemma40k
for r in ladder-100000 e3-mix40k gemma40k; do
  $PY training/valid_breakdown.py --data training/out/data-meta --model training/out/runs/$r/ckpt --json training/out/runs/$r/valid-breakdown.json > training/out/runs/$r/valid-breakdown.txt 2>&1
done
echo QUEUE1_DONE
