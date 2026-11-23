#!/usr/bin/env bash
# Behind queue-2: the local rank travels in each option's text (fmt tdr), trained on the same rows as had40k (exact example retrieved, no injection).
set -uo pipefail
cd "$(dirname "$0")/.."
while pgrep -f "queue-[12].sh" >/dev/null; do sleep 10; done
EPOCHS=1 training/run.sh train rank40k tdr 20 512 training/out/data-rank40k
cd training/out/runs/rank40k && for b in "" "-casual"; do ../../venv/bin/python ../../../evaluate.py --model ckpt --cache ../../eval-cands$b.json --fmt tdr --cands 20 --dump dump$b.json >/dev/null 2>&1; done
echo QUEUE3_DONE
