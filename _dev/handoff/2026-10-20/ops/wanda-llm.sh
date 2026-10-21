#!/usr/bin/env bash
# Wanda's local LLM on llama-server (127.0.0.1:8081, alias "bonsai"): Ternary-Bonsai-2-9B,
# a Qwen3.5 hybrid (8 of its 32 layers keep a KV cache) trained to 262 144 tokens.
#
#   cpu  while the GPU trains (default): 12 threads, 32k context, one request at a time
#        (measured: two at once only shared the same cores, 5.0 → 3.9 tokens/s each)
#   gpu  when the GPU is free: every layer on the RTX 3070, 64k context, 4 in parallel
#        (~4 GB of VRAM, leaving room for IRIS's ~2 GB)
#   gpu-max  the same with 128k context (~5 GB): only when IRIS is not on the GPU
#
# Both quantise the KV cache to q8_0 with flash attention, batch requests continuously and
# reuse a cached prompt prefix (--cache-reuse), so a follow-up question does not re-read it.
# Run: wsl -d ArchieOS-Operator-RTX3070-Test -- bash /mnt/c/Users/Penta/Desktop/ArchieOS/wanda-llm.sh [cpu|gpu]
set -euo pipefail
MODE=${1:-cpu}
MODEL=/home/archie/wanda/models/Ternary-Bonsai-2-9B-PTQ1_0.gguf
RUNTIME=/opt/archie/llama-cuda
LOG=/home/archie/wanda/llama-8081.log

common=(--model "$MODEL" --host 127.0.0.1 --port 8081 --alias bonsai --jinja --predict 512
        --flash-attn on --cache-type-k q8_0 --cache-type-v q8_0
        --cont-batching --kv-unified --cache-reuse 256 --cache-ram 1024)
case "$MODE" in
  cpu) env=(CUDA_VISIBLE_DEVICES=)
       args=(--device none --n-gpu-layers 0 --threads 12 --threads-batch 14
             --ctx-size 32768 --parallel 1 --batch-size 2048 --ubatch-size 512) ;;
  gpu|gpu-max)
       ctx=65536; [[ $MODE == gpu-max ]] && ctx=131072
       env=()
       args=(--n-gpu-layers 99 --threads 4 --threads-batch 8
             --ctx-size "$ctx" --parallel 4 --batch-size 2048 --ubatch-size 512) ;;
  *) echo "usage: $0 [cpu|gpu|gpu-max]" >&2; exit 2 ;;
esac

# Stop the previous server on this port only.
pkill -f -- "llama-server.*--port 8081" 2>/dev/null || true
for _ in $(seq 1 30); do pgrep -f -- "llama-server.*--port 8081" >/dev/null || break; sleep 1; done

env "${env[@]}" LD_LIBRARY_PATH="$RUNTIME/lib" setsid nohup "$RUNTIME/bin/llama-server" \
  "${common[@]}" "${args[@]}" >"$LOG" 2>&1 </dev/null &
for _ in $(seq 1 120); do
  if curl -sf -m 2 http://127.0.0.1:8081/health >/dev/null; then
    echo "llama-server $MODE ready on 127.0.0.1:8081 (log $LOG)"; exit 0
  fi
  sleep 1
done
echo "llama-server did not become healthy; see $LOG" >&2
tail -n 20 "$LOG" >&2
exit 1
