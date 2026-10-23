#!/bin/bash
# Runs N concurrent build-posture probes per routing mode; modes run one after another.
set -u
BIN=$1; OUT=$2; N=$3; shift 3
SCRIPT=/private/tmp/claude-501/-Volumes-Data-codes-ai-tui/c10d4b83-9eae-40f8-b969-a77bf33363c4/scratchpad/tool_loop_probe.py
mkdir -p "$OUT"
for mode in "$@"; do
  pids=()
  for i in $(seq 1 "$N"); do
    CHATGPT_ROUTING_DIAG=$mode python3 -I "$SCRIPT" --binary "$BIN" --label "$mode-c$i" --build-posture --output "$OUT" &
    pids+=($!)
  done
  for pid in "${pids[@]}"; do wait "$pid"; done
done
