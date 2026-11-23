#!/usr/bin/env bash
# Recipe levers on the best data/format so far (rank40k: rows with the exact example retrieved, local rank in the option text).
# One change per run: a second seed (how much noise?), encoder learning rate x0.5 and x2, a bigger token budget, the other loss.
set -uo pipefail
cd "$(dirname "$0")/.."
PY=training/out/venv/bin/python
D=training/out/data-rank40k
run() {  # name head "extra flags"
  local name="$1" head="$2" extra="$3"
  EPOCHS=1 TRAIN_ARGS="$extra" training/run.sh train "$name" tdr 20 "$head" "$D"
  ( cd "training/out/runs/$name" && for b in "" "-casual"; do ../../venv/bin/python ../../../evaluate.py --model ckpt --cache ../../eval-cands$b.json --fmt tdr --cands 20 --dump dump$b.json >/dev/null 2>&1; done )
  echo "RUN_DONE $name"
}
run rank40k-seed1 512 "--seed 1"
run rank-elr-half 512 "--encoder-lr 1.25e-5"
run rank-elr-x2 512 "--encoder-lr 5e-5"
run rank-h768 768 ""
run rank-rlcd 512 "--loss rlcd"
echo QUEUE4_DONE
