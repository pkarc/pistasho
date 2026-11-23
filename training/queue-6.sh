#!/usr/bin/env bash
# The last batch (then the plan closes): disagreement focus, seeds, freeze-encoder control, none-share, short/typo augmentation.
# All with fmt tdr, head 512, 1 epoch, the default recipe unless said.
set -uo pipefail
cd "$(dirname "$0")/.."
run() {  # name datadir "extra flags"
  local name="$1" d="$2" extra="$3"
  EPOCHS=1 TRAIN_ARGS="$extra" training/run.sh train "$name" tdr 20 512 "$d"
  ( cd "training/out/runs/$name" && for b in "" "-casual"; do ../../venv/bin/python ../../../evaluate.py --model ckpt --cache ../../eval-cands$b.json --fmt tdr --cands 20 --dump dump$b.json >/dev/null 2>&1; done )
  echo "RUN_DONE $name"
}
R=training/out
run dis-s0 $R/data-disagree "--seed 0"
run bal-s0 $R/data-balanced "--seed 0"
run dis-s1 $R/data-disagree "--seed 1"
run dis-s2 $R/data-disagree "--seed 2"
run rank40k-seed2 $R/data-rank40k "--seed 2"
run rank-freeze $R/data-rank40k "--freeze-encoder"
while ! grep -q GEN_DONE $R/gen-final.log 2>/dev/null; do sleep 10; done
run rank-n20 $R/data-n20-40k ""
run rank-aug $R/data-aug-40k ""
echo QUEUE6_DONE
