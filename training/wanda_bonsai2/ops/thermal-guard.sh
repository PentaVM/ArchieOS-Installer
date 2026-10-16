#!/bin/bash
# Thermal/duty-cycle guard for the Wanda worker. Only writes the worker's own
# control.json (pause/run); never signals a process and changes no GPU setting.
# - Duty cycle: RUN_S seconds of work, then REST_S seconds paused (slow, cool pace).
# - Over HOT_C: pause until the GPU is back at or below COOL_C.
# - nvidia-smi failure (GPU lost): pause and keep watching.
# - "stop" written by anyone is respected: the guard never overrides it and exits.
# Usage: thermal-guard.sh CUTOFF_ISO
set -u
ROOT=/home/archie/wanda/run
CUTOFF="$1"
HOT_C="${HOT_C:-75}"; COOL_C="${COOL_C:-60}"
RUN_S="${RUN_S:-180}"; REST_S="${REST_S:-45}"; POLL_S="${POLL_S:-3}"
LOG="$ROOT/thermal.log"
exec 9>"$ROOT/thermal.lock"
flock -n 9 || { echo "$(date -Is) guard already running" >> "$LOG"; exit 0; }
end=$(date -d "$CUTOFF" +%s)

current() { python3 -c 'import json,sys;print(json.load(open(sys.argv[1])).get("command","run"))' "$ROOT/control.json" 2>/dev/null || echo run; }
set_cmd() {
  [ "$(current)" = "$1" ] && return
  printf '{"command": "%s"}' "$1" > "$ROOT/control.json.tmp" && mv -f "$ROOT/control.json.tmp" "$ROOT/control.json"
  echo "$(date -Is) control=$1 ($2)" >> "$LOG"
}

echo "$(date -Is) guard start hot=$HOT_C cool=$COOL_C run=${RUN_S}s rest=${REST_S}s" >> "$LOG"
hot=0; phase=run; phase_start=$(date +%s); last_log=0
while [ "$(date +%s)" -lt "$end" ]; do
  [ "$(current)" = "stop" ] && { echo "$(date -Is) stop seen; guard exits" >> "$LOG"; exit 0; }
  now=$(date +%s)
  line=$(nvidia-smi --query-gpu=temperature.gpu,power.draw,utilization.gpu --format=csv,noheader,nounits 2>/dev/null | head -n1)
  temp=$(echo "$line" | cut -d, -f1 | tr -d ' ')
  if ! [[ "$temp" =~ ^[0-9]+$ ]]; then
    set_cmd pause "nvidia-smi unavailable"; sleep 30; continue
  fi
  if [ "$temp" -ge "$HOT_C" ]; then hot=1; elif [ "$temp" -le "$COOL_C" ]; then hot=0; fi
  if [ $((now - phase_start)) -ge "$( [ "$phase" = run ] && echo "$RUN_S" || echo "$REST_S")" ]; then
    phase=$( [ "$phase" = run ] && echo rest || echo run ); phase_start=$now
  fi
  if [ "$hot" -eq 1 ]; then set_cmd pause "hot ${temp}C"
  elif [ "$phase" = rest ]; then set_cmd pause "duty rest ${temp}C"
  else set_cmd run "duty run ${temp}C"; fi
  if [ $((now - last_log)) -ge 60 ]; then echo "$(date -Is) gpu $line phase=$phase hot=$hot" >> "$LOG"; last_log=$now; fi
  sleep "$POLL_S"
done
echo "$(date -Is) cutoff; guard exits" >> "$LOG"
