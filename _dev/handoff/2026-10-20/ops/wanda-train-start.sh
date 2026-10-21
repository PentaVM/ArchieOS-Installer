#!/usr/bin/env bash
# Starts or resumes the Wanda LNCM GPU training detached from any terminal, so closing a
# window, a tab or a session does not stop it (a closed tab ended the 20:40 run). Then starts
# the watcher that moves Wanda's LLM to the GPU when training ends.
#   wsl -d ArchieOS-Operator-RTX3070-Test -- bash /mnt/c/Users/Penta/Desktop/ArchieOS/wanda-train-start.sh
# Follow: ~/wanda-lncm/logs/run.out and ~/wanda-lncm/ckpt-small/train-log.jsonl
set -u
HERE=/mnt/c/Users/Penta/Desktop/ArchieOS
if pgrep -f '[t]raining/wanda_lncm/train-local.sh' >/dev/null; then
  echo "training already running"
else
  setsid -f bash "$HERE/wanda-train-cuda128.sh" >>/home/archie/wanda-lncm/logs/run.out 2>&1 </dev/null
  sleep 3
  pgrep -f '[t]raining/wanda_lncm/train-local.sh' >/dev/null && echo "training started" \
    || { echo "training did not start; see ~/wanda-lncm/logs/run.out"; exit 1; }
fi
if ! pgrep -f '[w]anda-llm-after-training.sh' >/dev/null; then
  setsid -f bash "$HERE/wanda-llm-after-training.sh" >/dev/null 2>&1 </dev/null
  echo "LLM-to-GPU watcher started"
fi
