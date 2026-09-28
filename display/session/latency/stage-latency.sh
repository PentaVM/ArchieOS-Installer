#!/usr/bin/env bash
# How fast the Archie Stage passes input on, measured on a minimal session: headless sway, the
# Stage and one probe window, nothing else (no browser, no Observatory), so it can run beside
# training for the minute it takes. Everything it starts is stopped at the end.
#
#   stage-latency.sh [count]     prints the gate round trip and the delivery to the window
#
# LOOKS=N also times N captures each way: a full-size PNG snapshot and a 512-wide raw frame.
# SEATS=shared starts the Stage with a seat for the agent beside the person's. HUMAN=1 (with
# SEATS=shared) also plays a person through VNC while the agent works: mouse moves and typed
# letters, which must reach the window without taking anything from the agent.
set -uo pipefail
QT_DIR=${QT_DIR:-/opt/Qt/6.11.2/gcc_64}
BUILD=${BUILD:-/workspace/build}
CORE=${CORE:-/workspace/archieos/core/target/release}
HERE=$(cd "$(dirname "$0")" && pwd)
RUN=$(mktemp -d /tmp/archie-latency.XXXX)
COUNT=${1:-40}
export XDG_RUNTIME_DIR=$RUN/xdg LD_LIBRARY_PATH=$QT_DIR/lib QT_PLUGIN_PATH=$QT_DIR/plugins QML_IMPORT_PATH=$QT_DIR/qml
mkdir -p "$XDG_RUNTIME_DIR" && chmod 700 "$RUN" "$XDG_RUNTIME_DIR"
pids=()
guest() { ( echo 900 > /proc/self/oom_score_adj; exec nice -n 5 "$@" ) & pids+=($!); }
cleanup() { for pid in "${pids[@]}"; do kill "$pid" 2>/dev/null; done; sleep 1; rm -rf "$RUN"; }
trap cleanup EXIT

printf 'output HEADLESS-1 resolution 1280x800\ndefault_border none\nfocus_follows_mouse no\n' > "$RUN/sway.conf"
WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 guest sway --unsupported-gpu -c "$RUN/sway.conf" > "$RUN/sway.log" 2>&1
for _ in $(seq 1 50); do ls "$XDG_RUNTIME_DIR"/wayland-? >/dev/null 2>&1 && break; sleep 0.2; done
export WAYLAND_DISPLAY=$(basename "$(ls "$XDG_RUNTIME_DIR"/wayland-? | head -1)")
"$CORE/archie-stage-cli" keygen "$RUN/key" >/dev/null
LIBGL_ALWAYS_SOFTWARE=1 QT_QPA_PLATFORM=wayland guest "$BUILD/stage/archie-stage" --socket-name archie-stage \
  --control "$RUN/control.sock" --key-file "$RUN/key" --tempo agent --seats "${SEATS:-exclusive}" > "$RUN/stage.log" 2>&1
for _ in $(seq 1 50); do [ -S "$RUN/control.sock" ] && [ -S "$XDG_RUNTIME_DIR/archie-stage" ] && break; sleep 0.2; done
WAYLAND_DISPLAY=archie-stage QT_QPA_PLATFORM=wayland LIBGL_ALWAYS_SOFTWARE=1 \
  guest "$QT_DIR/bin/qml" "$HERE/probe-window.qml" > "$RUN/window.log" 2>&1
APP=""
for _ in $(seq 1 40); do
  APP=$(timeout 2 "$CORE/archie-stage-cli" watch --socket "$RUN/control.sock" 2>/dev/null | grep -o '"app_id":"[^"]*"' | head -1 | cut -d'"' -f4)
  [ -n "$APP" ] && break; sleep 0.5
done
[ -n "$APP" ] || { echo "the probe window never appeared on the stage"; tail -5 "$RUN/stage.log" "$RUN/window.log"; exit 1; }
cat > "$RUN/policy.toml" <<P
[[app]]
app_id = "$APP"
display_name = "latency probe"
allow = ["move", "click", "key", "hold", "focus", "snapshot"]
max_text_grade = "public"
max_text_chars = 8
P
if [ "${HUMAN:-0}" = 1 ]; then
  # The person, through the same VNC a person uses: sway's output, which the Stage window fills.
  guest wayvnc 127.0.0.1 5909 > "$RUN/wayvnc.log" 2>&1
  [ -x /tmp/vncvenv/bin/vncdo ] || { python3 -m venv /tmp/vncvenv && /tmp/vncvenv/bin/pip -q install vncdotool; } >/dev/null 2>&1
  sleep 1
  # A person clicks into the window first (their own seat's focus), then types while moving.
  ( /tmp/vncvenv/bin/vncdo -s 127.0.0.1::5909 move 420 320 click 1 >> "$RUN/person.log" 2>&1
    for i in $(seq 1 30); do /tmp/vncvenv/bin/vncdo -s 127.0.0.1::5909 move $((300 + i * 10)) 300 type h pause 0.1 >> "$RUN/person.log" 2>&1; done ) &
  pids+=($!)
fi
"$CORE/archie-stage-latency" --socket "$RUN/control.sock" --key-file "$RUN/key" --policy "$RUN/policy.toml" \
  --app "$APP" --count "$COUNT" --out "$RUN/sent.jsonl" --looks "${LOOKS:-0}"
if [ "${HUMAN:-0}" = 1 ]; then
  sleep 3
  echo "letters in the window: person h=$(grep -c '"key":72' "$RUN/window.log"), agent w=$(grep -c '"key":87' "$RUN/window.log")"
  echo "authority lines in the stage log: $(grep -c 'authority' "$RUN/stage.log")"
  grep -m3 -i -e 'seat' -e 'authority' "$RUN/stage.log"
  echo "person's button presses in the window: $(grep -c '"e":"button_down"' "$RUN/window.log")"
  tail -3 "$RUN/person.log" 2>/dev/null; tail -2 "$RUN/wayvnc.log" 2>/dev/null
fi
sleep 1
# Delivery: for every executed action, the first matching event in the window's log after it was sent.
python3 - "$RUN/sent.jsonl" "$RUN/window.log" <<'PY'
import json, sys
sent = [json.loads(l) for l in open(sys.argv[1]) if l.strip()]
seen = []
for line in open(sys.argv[2]):
    at = line.find("{")
    if at >= 0:
        try: seen.append(json.loads(line[at:]))
        except ValueError: pass
want = {"key_down": "key_down", "key_up": "key_up", "move": "motion", "click": "button_down"}
used, delays = set(), {}
for item in sent:
    if not item["executed"]: continue
    for index, event in enumerate(seen):
        if index not in used and event["e"] == want[item["kind"]] and event["t"] >= item["t"]:
            used.add(index); delays.setdefault(item["kind"], []).append(event["t"] - item["t"]); break
for kind, values in sorted(delays.items()):
    values.sort()
    print(json.dumps({"delivered": kind, "n": len(values), "p50_ms": values[len(values)//2],
                      "p95_ms": values[int(0.95*(len(values)-1))], "max_ms": values[-1]}))
PY
