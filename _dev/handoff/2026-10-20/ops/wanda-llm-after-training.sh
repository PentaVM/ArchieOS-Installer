#!/usr/bin/env bash
# Waits for the Wanda LNCM training run (train-local.sh) to end, then moves Wanda's local LLM
# to the GPU (wanda-llm.sh gpu: 128k context, 4 requests in parallel). Started detached:
#   wsl -d ArchieOS-Operator-RTX3070-Test -- setsid nohup bash /mnt/c/Users/Penta/Desktop/ArchieOS/wanda-llm-after-training.sh >/dev/null 2>&1 &
set -u
LOG=/home/archie/wanda/llm-after-training.log
echo "$(date -Is) waiting for train-local.sh to end" >>"$LOG"
# The bracket keeps the pattern from matching a command line that merely quotes it.
if ! pgrep -f '[t]raining/wanda_lncm/train-local.sh' >/dev/null; then
  echo "$(date -Is) no training is running; nothing to wait for" >>"$LOG"
  exit 0
fi
while pgrep -f '[t]raining/wanda_lncm/train-local.sh' >/dev/null; do sleep 60; done
# Only a finished run frees the GPU for good; after a crash it will be restarted, and an LLM on
# the GPU would take the memory it needs (2026-10-20: the generator's OOM moved the LLM there).
if ! tail -n 5 /home/archie/wanda-lncm/logs/run.out | grep -q 'done: checkpoint'; then
  echo "$(date -Is) training stopped without finishing; the LLM stays where it is" >>"$LOG"
  exit 0
fi
echo "$(date -Is) training finished; moving the LLM to the GPU" >>"$LOG"
bash /mnt/c/Users/Penta/Desktop/ArchieOS/wanda-llm.sh gpu >>"$LOG" 2>&1 \
  || bash /mnt/c/Users/Penta/Desktop/ArchieOS/wanda-llm.sh cpu >>"$LOG" 2>&1
