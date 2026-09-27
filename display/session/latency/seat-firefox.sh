#!/usr/bin/env bash
# Does Firefox take input from two seats at once? A minimal session (headless sway, the Stage with
# --seats shared, Firefox on a local probe page), the agent pressing keys and clicking through its
# seat while a person clicks and types through VNC. Prints how many of each reached the page.
set -uo pipefail
QT_DIR=${QT_DIR:-/opt/Qt/6.11.2/gcc_64}
BUILD=${BUILD:-/workspace/build}
CORE=${CORE:-/workspace/archieos/core/target/release}
HERE=$(cd "$(dirname "$0")" && pwd)
RUN=$(mktemp -d /tmp/archie-seats.XXXX)
export XDG_RUNTIME_DIR=$RUN/xdg LD_LIBRARY_PATH=$QT_DIR/lib QT_PLUGIN_PATH=$QT_DIR/plugins QML_IMPORT_PATH=$QT_DIR/qml
mkdir -p "$XDG_RUNTIME_DIR" && chmod 700 "$RUN" "$XDG_RUNTIME_DIR"
pids=()
guest() { ( echo 900 > /proc/self/oom_score_adj; exec nice -n 5 "$@" ) & pids+=($!); }
cleanup() { for pid in "${pids[@]}"; do kill "$pid" 2>/dev/null; done; sleep 2; rm -rf "$RUN"; }
trap cleanup EXIT
printf 'output HEADLESS-1 resolution 1280x800\ndefault_border none\nfocus_follows_mouse no\n' > "$RUN/sway.conf"
WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 guest sway --unsupported-gpu -c "$RUN/sway.conf" > "$RUN/sway.log" 2>&1
for _ in $(seq 1 50); do ls "$XDG_RUNTIME_DIR"/wayland-? >/dev/null 2>&1 && break; sleep 0.2; done
export WAYLAND_DISPLAY=$(basename "$(ls "$XDG_RUNTIME_DIR"/wayland-? | head -1)")
"$CORE/archie-stage-cli" keygen "$RUN/key" >/dev/null
LIBGL_ALWAYS_SOFTWARE=1 QT_QPA_PLATFORM=wayland guest "$BUILD/stage/archie-stage" --socket-name archie-stage \
  --control "$RUN/control.sock" --key-file "$RUN/key" --tempo agent --seats "${SEATS:-shared}" > "$RUN/stage.log" 2>&1
for _ in $(seq 1 50); do [ -S "$RUN/control.sock" ] && [ -S "$XDG_RUNTIME_DIR/archie-stage" ] && break; sleep 0.2; done
mkdir -p "$RUN/ff"
firefox --headless -CreateProfile "seats $RUN/ff" >/dev/null 2>&1 || true
cat > "$RUN/ff/user.js" <<P
user_pref("devtools.console.stdout.content", true);
user_pref("browser.sessionstore.resume_from_crash", false);
user_pref("browser.startup.homepage_override.mstone", "ignore");
user_pref("datareporting.policy.dataSubmissionEnabled", false);
P
WAYLAND_DISPLAY=archie-stage MOZ_ENABLE_WAYLAND=1 LIBGL_ALWAYS_SOFTWARE=1 \
  guest firefox --profile "$RUN/ff" --new-window "file://$HERE/probe-page.html" > "$RUN/firefox.log" 2>&1
for _ in $(seq 1 60); do
  timeout 2 "$CORE/archie-stage-cli" watch --socket "$RUN/control.sock" 2>/dev/null | grep -q '"app_id":"firefox"' && break; sleep 1
done
sleep 8   # the page loads
cat > "$RUN/policy.toml" <<P
[[app]]
app_id = "firefox"
display_name = "seat probe"
allow = ["move", "click", "key", "hold", "focus", "snapshot"]
max_text_grade = "public"
max_text_chars = 8
P
guest wayvnc 127.0.0.1 5909 > "$RUN/wayvnc.log" 2>&1
[ -x /tmp/vncvenv/bin/vncdo ] || { python3 -m venv /tmp/vncvenv && /tmp/vncvenv/bin/pip -q install vncdotool; } >/dev/null 2>&1
sleep 1
( /tmp/vncvenv/bin/vncdo -s 127.0.0.1::5909 move 420 200 click 1 >> "$RUN/person.log" 2>&1
  for i in $(seq 1 20); do /tmp/vncvenv/bin/vncdo -s 127.0.0.1::5909 type h pause 0.2 >> "$RUN/person.log" 2>&1; done ) &
pids+=($!)
"$CORE/archie-stage-latency" --socket "$RUN/control.sock" --key-file "$RUN/key" --policy "$RUN/policy.toml" \
  --app firefox --count 20 --out "$RUN/sent.jsonl" --snapshot "${SNAPSHOT:-/tmp/seat-firefox.png}"
sleep 4
echo "firefox saw: person h=$(grep -c '"key":"h"' "$RUN/firefox.log") (keydowns+keyups), agent w=$(grep -c '"key":"w"' "$RUN/firefox.log"), presses=$(grep -c '"e":"button_down"' "$RUN/firefox.log")"
echo "authority lines: $(grep -c authority "$RUN/stage.log")"
grep -m2 -i -e error -e crash "$RUN/firefox.log" | cut -c1-160; echo "--- firefox log head"; grep -v "^[[:space:]]*$" "$RUN/firefox.log" | head -8 | cut -c1-160
