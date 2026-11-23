#!/usr/bin/env bash
# Behind queue-1: does training only on rows where the right example was retrieved (no injection) transfer better?
set -uo pipefail
cd "$(dirname "$0")/.."
while pgrep -f "queue-1.sh" >/dev/null; do sleep 10; done
PY=training/out/venv/bin/python
$PY training/subset_by_source.py training/out/data-meta2 40000 training/out/data-had40k gen-gemma gen-gemma-b nl2bash tldr-en tldr-es tldr-mix had
EPOCHS=1 training/run.sh train had40k td 20 512 training/out/data-had40k
cd training/out/runs/had40k && for b in "" "-casual"; do ../../venv/bin/python ../../../evaluate.py --model ckpt --cache ../../eval-cands$b.json --fmt td --cands 20 --dump dump$b.json >/dev/null 2>&1; done; cd ../../../..
$PY training/valid_breakdown.py --data training/out/data-meta2 --model training/out/runs/had40k/ckpt --json training/out/runs/had40k/valid-breakdown.json > training/out/runs/had40k/valid-breakdown.txt 2>&1
echo QUEUE2_DONE
