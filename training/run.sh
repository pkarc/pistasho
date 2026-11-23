#!/usr/bin/env bash
# Training pipeline for the local decision model (Laya, fine-tuned on pistasho's task).
#
# Split in two places on purpose:
#   * the machine with pistasho and its database makes the data:    run.sh data, run.sh pack
#   * the machine with the GPU (an RTX 5090) trains and exports:    run.sh setup, run.sh grid, run.sh export
#
#   training/run.sh setup               venv + torch (CUDA 12.8 build) + laya + onnx   (GPU machine)
#   training/run.sh data                tldr -> train/valid JSONL for each option format (needs pistasho + its DB)
#   training/run.sh pack                data + eval candidates -> training/out/bundle.tar.gz to copy to the GPU machine
#   training/run.sh unpack bundle.tar.gz   (GPU machine)
#   training/run.sh baseline            the untouched checkpoint on the eval banks (zero-shot reference)
#   training/run.sh smoke               200 rows on the GPU: rows per second, memory, no NaN (first thing to run there)
#   training/run.sh ladder              the same recipe with 10k, 40k and 100k rows ($SIZES): does more data help?
#   training/run.sh grid                trains and evaluates every variant in $VARIANTS (GPU machine, hours)
#   training/run.sh train NAME FMT CANDS HEAD    one variant
#   training/run.sh export NAME FMT CANDS   pruned ONNX of a trained variant + parity and size check
#
# Variables: OUT (default training/out), EPOCHS, MICRO_BATCH, GRAD_ACCUM, PY (python), VARIANTS.
set -euo pipefail
cd "$(dirname "$0")/.."

OUT="${OUT:-training/out}"
PY="${PY:-$OUT/venv/bin/python}"
EPOCHS="${EPOCHS:-3}"
MICRO_BATCH="${MICRO_BATCH:-16}"
GRAD_ACCUM="${GRAD_ACCUM:-4}"
# NAME:FMT:CANDS:HEAD_MAX_LEN  (HEAD_MAX_LEN is the token budget for all the options together)
VARIANTS="${VARIANTS:-td20h256:td:20:256 td20h512:td:20:512 tdc20h768:tdc:20:768 td10h256:td:10:256}"
SIZES="${SIZES:-10000 40000 100000}"
# data stage: GEN = generated-question files, NL2BASH = import_nl2bash.py output, both optional
GEN="${GEN:-}"; NL2BASH="${NL2BASH:-}"; MAX_PER_TOOL="${MAX_PER_TOOL:-150}"
BASE="${BASE:-multilingual}"

stage_setup() {
  mkdir -p "$OUT"
  python3 -m venv "$OUT/venv"
  if command -v nvidia-smi >/dev/null; then
    # Blackwell (RTX 50xx, sm_120) needs the CUDA 12.8 wheels
    "$OUT/venv/bin/pip" install -q torch --index-url https://download.pytorch.org/whl/cu128
  else
    "$OUT/venv/bin/pip" install -q torch --index-url https://download.pytorch.org/whl/cpu
  fi
  "$OUT/venv/bin/pip" install -q -r training/requirements.txt
  "$PY" - <<'PY'
import torch
print("torch", torch.__version__, "cuda", torch.cuda.is_available())
if torch.cuda.is_available():
    print(torch.cuda.get_device_name(0), "capability", torch.cuda.get_device_capability(0),
          "VRAM GB", round(torch.cuda.get_device_properties(0).total_memory / 1e9, 1))
PY
}

stage_data() {
  for fmt in td tdc; do
    # shellcheck disable=SC2086
    "$PY" training/make_data.py --out "$OUT/data-$fmt" --fmt "$fmt" --cands 20 --workers "${WORKERS:-8}" \
      --max-per-tool "$MAX_PER_TOOL" ${GEN:+--generated $GEN} ${NL2BASH:+--toollevel $NL2BASH}
  done
  "$PY" training/make_eval_cache.py "$OUT/eval-cands.json"
  "$PY" training/make_eval_cache.py "$OUT/eval-cands-casual.json" --casual
}

stage_pack() {
  tar czf "$OUT/bundle.tar.gz" -C "$OUT" \
    data-td/train.jsonl data-td/valid.jsonl data-tdc/train.jsonl data-tdc/valid.jsonl eval-cands.json eval-cands-casual.json
  ls -la "$OUT/bundle.tar.gz"
}

stage_unpack() { mkdir -p "$OUT" && tar xzf "$1" -C "$OUT" && ls "$OUT"; }

stage_baseline() {
  "$PY" training/evaluate.py --model convaiinnovations/laya --subfolder "$BASE" --cache "$OUT/eval-cands.json" \
    --fmt td --cands 20 --json "$OUT/baseline.json" 2>&1 | grep -v -i warn
}

stage_train() {
  local name="$1" fmt="$2" cands="$3" head="$4" data="${5:-$OUT/data-$2}"
  local run="$OUT/runs/$name"
  mkdir -p "$run"
  local started rows
  started=$(date +%s); rows=$(wc -l <"$data/train.jsonl")
  # the sequence is the options' budget plus room for the question and the instructions
  "$PY" -m laya.train_cli --data "$data/train.jsonl" --eval "$data/valid.jsonl" --base "$BASE" \
    --out "$run/ckpt" --epochs "$EPOCHS" --micro-batch "$MICRO_BATCH" --grad-accum "$GRAD_ACCUM" \
    --head-max-len "$head" --max-len "$((head + 256))" --loss soft-ce --shuffle-options --device auto \
    2>&1 | tee "$run/train.log"
  echo "trained $rows rows x $EPOCHS epochs in $(( $(date +%s) - started )) s" | tee -a "$run/train.log"
  for bank in "" "-casual"; do
    [ -f "$OUT/eval-cands$bank.json" ] || continue
    echo "######## eval banks${bank:- as written}" | tee -a "$run/eval.txt"
    "$PY" training/evaluate.py --model "$run/ckpt" --cache "$OUT/eval-cands$bank.json" --fmt "$fmt" --cands "$cands" \
      --json "$run/eval$bank.json" 2>&1 | grep -v -i warn | tee -a "$run/eval.txt"
  done
}

stage_smoke() {
  "$PY" - <<'PY'
import torch
assert torch.cuda.is_available(), "no CUDA: this is meant for the GPU machine"
print(torch.cuda.get_device_name(0), torch.cuda.get_device_capability(0))
PY
  "$PY" training/subsample.py "$OUT/data-td" 200 "$OUT/data-smoke"
  EPOCHS=1 stage_train smoke td 20 512 "$OUT/data-smoke"
  nvidia-smi --query-gpu=memory.used,memory.total --format=csv 2>/dev/null || true
}

stage_ladder() {
  for n in $SIZES; do
    "$PY" training/subsample.py "$OUT/data-td" "$n" "$OUT/data-ladder-$n"
    EPOCHS=1 stage_train "ladder-$n" td 20 512 "$OUT/data-ladder-$n"
  done
  "$PY" training/summary.py "$OUT"
}

stage_grid() {
  for v in $VARIANTS; do
    IFS=: read -r name fmt cands head <<<"$v"
    echo "=============== $name (fmt $fmt, $cands candidates, head $head) ==============="
    stage_train "$name" "$fmt" "$cands" "$head"
  done
  "$PY" training/summary.py "$OUT"
}

stage_export() {
  local name="$1" fmt="$2" cands="$3" run="$OUT/runs/$1"
  "$PY" training/export_onnx.py --ckpt "$run/ckpt" --out "$run/onnx" --cache "$OUT/eval-cands.json" \
    --fmt "$fmt" --cands "$cands" --prune-from "$OUT/data-$fmt/train.jsonl" "$OUT/data-$fmt/valid.jsonl" --no-int8
}

cmd="${1:-}"; shift || true
case "$cmd" in
  setup) stage_setup ;;
  data) stage_data ;;
  pack) stage_pack ;;
  unpack) stage_unpack "$@" ;;
  baseline) stage_baseline ;;
  smoke) stage_smoke ;;
  ladder) stage_ladder ;;
  train) stage_train "$@" ;;
  grid) stage_grid ;;
  export) stage_export "$@" ;;
  *) sed -n '2,22p' "$0"; exit 1 ;;
esac
