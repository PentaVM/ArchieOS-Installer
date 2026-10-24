#!/bin/bash
# Start the owned Wanda worker if it is not running, then hold this WSL session open
# for HOLD seconds so the distro stays up. Never kills or signals any process.
# Usage: ensure-worker.sh CUTOFF_ISO [HOLD_SECONDS]
set -u
PKG="$(cd "$(dirname "$0")/.." && pwd)"
CONFIG="$PKG/configs/rtx3070-development.toml"
ROOT=/home/archie/wanda/run
HEADERS=/mnt/c/Users/Penta/Archie-RTX3070-Handoff/local-verification/prism-b10777-source
CUTOFF="$1"; HOLD="${2:-240}"
mkdir -p "$ROOT"
if [ "$(date +%s)" -ge "$(date -d "$CUTOFF" +%s)" ]; then
  echo "$(date -Is) cutoff reached; not starting" >> "$ROOT/watchdog.log"; exit 0
fi
# The thermal guard always runs before the worker (it holds its own lock, so this is a
# no-op when it is already running).
if flock -n "$ROOT/thermal.lock" -c true 2>/dev/null; then
  echo "$(date -Is) thermal guard not running; starting" >> "$ROOT/watchdog.log"
  setsid nohup bash "$PKG/ops/thermal-guard.sh" "$CUTOFF" > /dev/null 2>&1 < /dev/null &
  sleep 5
fi
if flock -n "$ROOT/worker.lock" -c true 2>/dev/null; then
  echo "$(date -Is) worker not running; starting" >> "$ROOT/watchdog.log"
  cd "$PKG"
  PYTHONPATH="$PKG/src" setsid nohup /opt/archie/iris-venv/bin/python -m archie_wanda.cli worker \
    --config "$CONFIG" --headers "$HEADERS" --cutoff "$CUTOFF" >> "$ROOT/worker.log" 2>&1 < /dev/null &
fi
sleep "$HOLD"
