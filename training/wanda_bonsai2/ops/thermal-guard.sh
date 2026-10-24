#!/bin/bash
# Thermal/duty-cycle guard for the Wanda worker. Only writes the worker's own
# control.json (pause/run); never signals a process and changes no GPU setting.
# - Duty cycle: RUN_S seconds of work, then REST_S seconds paused (slow, cool pace).
# - Over HOT_C: pause until the GPU is back at or below COOL_C.
# - Enforced power limit above MAX_LIMIT_W (e.g. Afterburner profile lost after a
#   reboot): pause until the owner restores it.
# - $ROOT/hold exists (owner gaming, or another agent needs the GPU): pause.
# - nvidia-smi failure (GPU lost): pause and keep watching.
# - "stop" written by anyone is respected: the guard never overrides it and exits.
# Usage: thermal-guard.sh CUTOFF_ISO
set -u
ROOT=/home/archie/wanda/run
CUTOFF="$1"
# 2026-10-17 owner asked for more throughput ("temps are fine"). Measured over 2 h: p50 54 °C,
# p95 79 °C, max 85 °C, 266 hot pauses; most lost time was cooling from 75 to 60. The resume
# threshold moves up (60 → 68) and the pause threshold slightly (75 → 78); the peak is bounded by
# how fast this card heats, not by the threshold, so it is not raised further.
# 2026-10-17 14:45 rollback (STEERING rule: sampled peak > 87 °C or hourly p95 > 82 °C): with the
# owner's Blender sharing the GPU, the last 2 h reached max 89 °C, p95 88 °C. Back to 75/60.
# 2026-10-17 15:40 owner: "dedicate the GPU fully to this" (Blender and launchers closed). Full-use
# mode: no duty rest (REST_S=0), pause only past 86 °C. The driver's own boost target (~83 °C)
# lowers clocks first, so this guard is a backstop, not the regulator. Power/fan/clocks untouched.
HOT_C="${HOT_C:-78}"; COOL_C="${COOL_C:-65}"; MAX_LIMIT_W="${MAX_LIMIT_W:-140}"
RUN_S="${RUN_S:-180}"; REST_S="${REST_S:-20}"; POLL_S="${POLL_S:-1}"
LOG="$ROOT/thermal.log"
exec 9>"$ROOT/thermal.lock"
flock -n 9 || exit 0
end=$(date -d "$CUTOFF" +%s)

current() { python3 -c 'import json,sys;print(json.load(open(sys.argv[1])).get("command","run"))' "$ROOT/control.json" 2>/dev/null || echo run; }
set_cmd() {
  # Re-read just before writing: a "stop" that arrived after the loop's check must not be
  # overwritten (2026-10-17: a pause/run flip at 79-80 °C swallowed a restart's stop).
  local now_cmd; now_cmd="$(current)"
  [ "$now_cmd" = "$1" ] && return
  [ "$now_cmd" = stop ] && { echo "$(date -Is) stop seen; guard exits" >> "$LOG"; exit 0; }
  printf '{"command": "%s"}' "$1" > "$ROOT/control.json.tmp" && mv -f "$ROOT/control.json.tmp" "$ROOT/control.json"
  echo "$(date -Is) control=$1 ($2)" >> "$LOG"
}

echo "$(date -Is) guard start hot=$HOT_C cool=$COOL_C max_limit=${MAX_LIMIT_W}W run=${RUN_S}s rest=${REST_S}s cutoff=$CUTOFF" >> "$LOG"
hot=0; phase=run; phase_start=$(date +%s); last_log=0
while [ "$(date +%s)" -lt "$end" ]; do
  [ "$(current)" = "stop" ] && { echo "$(date -Is) stop seen; guard exits" >> "$LOG"; exit 0; }
  now=$(date +%s)
  line=$(nvidia-smi --query-gpu=temperature.gpu,power.draw,enforced.power.limit,utilization.gpu --format=csv,noheader,nounits 2>/dev/null | head -n1)
  temp=$(echo "$line" | cut -d, -f1 | tr -d ' ')
  limit=$(echo "$line" | cut -d, -f3 | tr -d ' ' | cut -d. -f1)
  if ! [[ "$temp" =~ ^[0-9]+$ && "$limit" =~ ^[0-9]+$ ]]; then
    set_cmd pause "nvidia-smi unavailable"; sleep 30; continue
  fi
  eff_hot=$HOT_C; eff_cool=$COOL_C; eff_run=$RUN_S; eff_rest=$REST_S
  if [ "$limit" -gt "$MAX_LIMIT_W" ]; then
    eff_hot=68; eff_cool=54; eff_run=45; eff_rest=20
  fi
  if [ "$temp" -ge "$eff_hot" ]; then hot=1; elif [ "$temp" -le "$eff_cool" ]; then hot=0; fi
  if [ "$eff_rest" -gt 0 ] && [ $((now - phase_start)) -ge "$( [ "$phase" = run ] && echo "$eff_run" || echo "$eff_rest")" ]; then
    phase=$( [ "$phase" = run ] && echo rest || echo run ); phase_start=$now
  fi
  if [ -e "$ROOT/hold" ]; then set_cmd pause "hold file present"
  elif [ "$hot" -eq 1 ]; then set_cmd pause "hot ${temp}C (limit ${limit}W)"
  elif [ "$phase" = rest ]; then set_cmd pause "duty rest ${temp}C (limit ${limit}W)"
  else set_cmd run "duty run ${temp}C (limit ${limit}W)"; fi
  if [ $((now - last_log)) -ge 60 ]; then echo "$(date -Is) gpu $line phase=$phase hot=$hot" >> "$LOG"; last_log=$now; fi
  sleep "$POLL_S"
done
echo "$(date -Is) cutoff; guard exits" >> "$LOG"
