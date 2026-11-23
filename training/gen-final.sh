#!/usr/bin/env bash
# Data for the last batch (CPU only): none-share 0.20 and the short/typo augmentation, both in the `tdr` format, 'had' rows, 40k.
set -euo pipefail
cd "$(dirname "$0")/.."
K=$PWD/training/out/kit
export PISTASHO_BIN=$K/pistasho XDG_DATA_HOME=$K/data XDG_CONFIG_HOME=$K/config HISTFILE=/dev/null PISTASHO_EMBEDDINGS=1
PY=training/out/venv/bin/python
G="--fmt tdr --cands 20 --workers 8 --max-per-tool 150 --generated $K/sources/gemma.jsonl $K/sources/gemma-b.jsonl --toollevel $K/sources/nl2bash.jsonl"
SRC="gen-gemma gen-gemma-b nl2bash tldr-en tldr-es tldr-mix had"
mkdir -p training/out/data-rank-n20 && ln -sf $K/sources/candidates.jsonl training/out/data-rank-n20/candidates.jsonl
$PY training/make_data.py --out training/out/data-rank-n20 --none-share 0.20 $G
$PY training/subset_by_source.py training/out/data-rank-n20 40000 training/out/data-n20-40k $SRC
# the augmented questions need new retrieval, so this one gets its own copy of the candidate cache
mkdir -p training/out/data-aug && cp $K/sources/candidates.jsonl training/out/data-aug/candidates.jsonl
$PY training/make_data.py --out training/out/data-aug --augment 0.4 $G
$PY training/subset_by_source.py training/out/data-aug 40000 training/out/data-aug-40k $SRC
echo GEN_DONE
