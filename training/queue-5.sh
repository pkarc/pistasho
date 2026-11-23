#!/usr/bin/env bash
# Does the rank format scale? All rows whose exact example was retrieved (96 911), 1 epoch, default recipe.
set -uo pipefail
cd "$(dirname "$0")/.."
PY=training/out/venv/bin/python
$PY training/subset_by_source.py training/out/data-rank 200000 training/out/data-rank-all gen-gemma gen-gemma-b nl2bash tldr-en tldr-es tldr-mix had
EPOCHS=1 training/run.sh train rank-all tdr 20 512 training/out/data-rank-all
( cd training/out/runs/rank-all && for b in "" "-casual"; do ../../venv/bin/python ../../../evaluate.py --model ckpt --cache ../../eval-cands$b.json --fmt tdr --cands 20 --dump dump$b.json >/dev/null 2>&1; done )
echo QUEUE5_DONE
