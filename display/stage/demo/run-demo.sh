#!/bin/sh
# Archie works an application while nobody touches the mouse or the keyboard.
#
# Starts the stage, opens the demo form inside it, and then, through the same policy gate,
# one-operation lease and sealed control socket the agent uses, clicks the field, types a
# note and presses "Guardar". The form's own log says whether the note was saved; nothing
# is judged from pixels.
#
# Move the mouse over the stage while it runs and the next action is refused with
# operator_has_control: that is the takeover working, not the demo failing.
#
# Environment (all optional):
#   STAGE         archie-stage binary                      default: archie-stage
#   STAGE_CLI     archie-stage-cli binary                  default: archie-stage-cli
#   QML           Qt's QML runtime                         default: /usr/lib/qt6/bin/qml
#   HOST_WAYLAND  session the stage window opens in        default: $WAYLAND_DISPLAY
#                 (WSLg: /mnt/wslg/runtime-dir/wayland-0)
#   RUN_DIR       key, sockets, logs and grabs             default: /tmp/archie-stage-demo
#   NOTE          what Archie types                        default: Archie escribió esto solo
#   GRABS=1       save the stage as PNG every 250 ms into $RUN_DIR/grabs
set -eu

STAGE=${STAGE:-archie-stage}
STAGE_CLI=${STAGE_CLI:-archie-stage-cli}
QML=${QML:-/usr/lib/qt6/bin/qml}
HOST_WAYLAND=${HOST_WAYLAND:-${WAYLAND_DISPLAY:-}}
RUN_DIR=${RUN_DIR:-/tmp/archie-stage-demo}
NOTE=${NOTE:-Archie escribió esto solo}
APP=org.qt-project.qml
HERE=$(cd "$(dirname "$0")" && pwd)

[ -n "$HOST_WAYLAND" ] || { echo "run-demo: set HOST_WAYLAND to the Wayland session to open the stage in" >&2; exit 1; }

mkdir -p "$RUN_DIR/xdg" "$RUN_DIR/grabs"
chmod 700 "$RUN_DIR" "$RUN_DIR/xdg"
# A socket left by an earlier run would be found at once and refuse the form's connection.
rm -f "$RUN_DIR"/grabs/*.png "$RUN_DIR/control.sock" "$RUN_DIR/xdg/archie-stage-demo"
export XDG_RUNTIME_DIR="$RUN_DIR/xdg"
[ -f "$RUN_DIR/key" ] || "$STAGE_CLI" keygen "$RUN_DIR/key" >/dev/null

# Only the demo form may be operated, and only by moving, clicking, typing and focusing.
cat > "$RUN_DIR/apps.toml" <<'EOF'
[[app]]
app_id = "org.qt-project.qml"
display_name = "Formulario de prueba"
allow = ["move", "click", "text", "focus"]
max_text_grade = "internal"
max_text_chars = 128
EOF

set -- --windowed --width 1600 --height 900 --socket-name archie-stage-demo \
       --control "$RUN_DIR/control.sock" --key-file "$RUN_DIR/key"
if [ "${GRABS:-0}" = 1 ]; then
    set -- "$@" --grab "$RUN_DIR/grabs/stage.png" --grab-after-ms 0 --grab-every-ms 250
fi
WAYLAND_DISPLAY="$HOST_WAYLAND" QT_QPA_PLATFORM=wayland "$STAGE" "$@" > "$RUN_DIR/stage.log" 2>&1 &
STAGE_PID=$!
FORM_PID=
cleanup() {
    [ -n "$FORM_PID" ] && kill "$FORM_PID" 2>/dev/null || true
    kill "$STAGE_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

# The control socket opens before the scene loads; applications can connect once the
# Wayland socket exists too.
for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
    [ -S "$RUN_DIR/control.sock" ] && [ -S "$XDG_RUNTIME_DIR/archie-stage-demo" ] && break
    sleep 0.5
done
[ -S "$XDG_RUNTIME_DIR/archie-stage-demo" ] || { echo "run-demo: the stage did not start; see $RUN_DIR/stage.log" >&2; exit 1; }

WAYLAND_DISPLAY=archie-stage-demo QT_QPA_PLATFORM=wayland QT_QUICK_BACKEND=software \
    "$QML" "$HERE/form.qml" > "$RUN_DIR/form.log" 2>&1 &
FORM_PID=$!
sleep 3  # the form maps, the stage tiles it, the form settles at its tiled size

layout=$(grep "form: layout" "$RUN_DIR/form.log" | tail -n 1)
[ -n "$layout" ] || { echo "run-demo: the form never reported its layout; see $RUN_DIR/form.log" >&2; exit 1; }
field_x=$(echo "$layout" | awk '{for (i = 1; i <= NF; i++) if ($i == "field") print $(i + 1)}')
field_y=$(echo "$layout" | awk '{for (i = 1; i <= NF; i++) if ($i == "field") print $(i + 2)}')
button_x=$(echo "$layout" | awk '{for (i = 1; i <= NF; i++) if ($i == "button") print $(i + 1)}')
button_y=$(echo "$layout" | awk '{for (i = 1; i <= NF; i++) if ($i == "button") print $(i + 2)}')

act() {
    "$STAGE_CLI" act --socket "$RUN_DIR/control.sock" --key-file "$RUN_DIR/key" \
        --policy "$RUN_DIR/apps.toml" --app "$APP" "$@"
}

echo "== Archie clicks the field at ($field_x, $field_y)"
act click "$field_x" "$field_y"
echo "== Archie types a note"
act type "$NOTE"
echo "== Archie presses Guardar at ($button_x, $button_y)"
act click "$button_x" "$button_y"
sleep 2  # the caption and the camera settle, for anyone watching

if grep -q "form: saved" "$RUN_DIR/form.log"; then
    echo "== the form saved: $(grep "form: saved" "$RUN_DIR/form.log" | tail -n 1 | sed 's/.*form: saved //')"
else
    echo "== the form did not save anything; see $RUN_DIR/form.log" >&2
    exit 1
fi
