#!/usr/bin/env bash
# ArchieOS on a machine with no screen, seen from a browser: a headless sway session with
# the Observatory (World, Forge, Orb) and the Archie Stage in it, served by wayvnc and
# noVNC on loopback. Started when wanted and stopped after, so it never runs beside
# training for longer than it is looked at.
#
#   archie-session.sh up      start everything (idempotent)
#   archie-session.sh down    stop everything
#   archie-session.sh status  what is running
#
# From the laptop:  ssh -L 6080:127.0.0.1:6080 <host>   then  http://localhost:6080 (the gate)
#
# Environment: QT_DIR (default /opt/Qt/6.11.2/gcc_64), BUILD (default /workspace/build),
# CORE (default /workspace/archieos/core/target/release), RUN (default /tmp/archie-session),
# SIZE (default 1920x1080).
set -euo pipefail
QT_DIR=${QT_DIR:-/opt/Qt/6.11.2/gcc_64}
BUILD=${BUILD:-/workspace/build}
CORE=${CORE:-/workspace/archieos/core/target/release}
RUN=${RUN:-/tmp/archie-session}
SIZE=${SIZE:-1920x1080}
ROOT=$RUN/root
export XDG_RUNTIME_DIR=$RUN/xdg
export LD_LIBRARY_PATH=$QT_DIR/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}
export QT_PLUGIN_PATH=$QT_DIR/plugins QML_IMPORT_PATH=$QT_DIR/qml QML2_IMPORT_PATH=$QT_DIR/qml

running() { [ -f "$RUN/$1.pid" ] && kill -0 "$(cat "$RUN/$1.pid")" 2>/dev/null; }

start() { # name, then the command; logs to $RUN/<name>.log
  local name=$1; shift
  running "$name" && return 0
  # The session is a guest beside training: first in the OOM killer's line.
  ( echo 900 > /proc/self/oom_score_adj; exec nice -n 5 "$@" ) > "$RUN/$name.log" 2>&1 &
  echo $! > "$RUN/$name.pid"
}

up() {
  mkdir -p "$XDG_RUNTIME_DIR" && chmod 700 "$RUN" "$XDG_RUNTIME_DIR"
  cat > "$RUN/sway.conf" <<EOF
output HEADLESS-1 resolution $SIZE
default_border none
focus_follows_mouse no
workspace 1 output HEADLESS-1
EOF
  # Software rendering: the container's NVIDIA driver has no GBM for a compositor, and
  # clients draw with Mesa's llvmpipe into shared memory, which pixman composes.
  WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 \
    start sway sway --unsupported-gpu -c "$RUN/sway.conf"
  for _ in $(seq 1 50); do ls "$XDG_RUNTIME_DIR"/wayland-* >/dev/null 2>&1 && break; sleep 0.2; done
  export WAYLAND_DISPLAY=$(basename "$(ls "$XDG_RUNTIME_DIR"/wayland-? | head -1)")
  echo "$WAYLAND_DISPLAY" > "$RUN/wayland-display"
  # VNC on loopback; archie-arena-gate (always on, 127.0.0.1:6080) serves noVNC and carries
  # its WebSocket to this.
  start wayvnc wayvnc 127.0.0.1 5901
  # The ArchieOS tree of this session, and the read-only bridge the Observatory draws from.
  mkdir -p "$ROOT/etc" "$ROOT/var/lib" "$ROOT/var/log" "$ROOT/run" && chmod 700 "$ROOT/run"
  # archied keeps its state in PostgreSQL 18 with TimescaleDB 2.29.2 and pgvector 0.8.6: a
  # cluster of this session's own, on loopback, started and stopped with it. Its connection
  # is $ROOT/etc/database.json (owner-only), written once by the setup.
  pg_ctlcluster 18 archie start 2>/dev/null || true
  start archied "$CORE/archied" --root "$ROOT"
  start displayd "$CORE/archie-displayd" --root "$ROOT" --orb-socket "$RUN/orb.sock"
  # The Observatory, with Mesa's software GL: Qt Quick 3D needs a GL context, and the
  # headless session has no hardware path to share buffers with.
  LIBGL_ALWAYS_SOFTWARE=1 QT_QPA_PLATFORM=wayland \
    start observatory "$BUILD/native/archie-display" --scene "${SCENE:-orb}"
  # The Archie Stage beside it: the compositor Archie works in, its control socket sealed
  # with this session's key, at the agent's tempo (the pointer's paths on a quarter of the
  # clock; TEMPO=person to watch it at a person's pace) and lines at 4000 px/s rather than a
  # hand's 900 (STROKE_SPEED): straight strokes lose nothing, and in Archie Arena covering a
  # canvas took 11 s of a 60 s run. Applications opened on it use its own Wayland socket.
  [ -f "$RUN/key" ] || "$CORE/archie-stage-cli" keygen "$RUN/key" >/dev/null
  if ! running stage; then rm -f "$RUN/control.sock" "$XDG_RUNTIME_DIR/archie-stage"; fi
  LIBGL_ALWAYS_SOFTWARE=1 QT_QPA_PLATFORM=wayland \
    start stage "$BUILD/stage/archie-stage" --socket-name archie-stage --control "$RUN/control.sock" \
      --key-file "$RUN/key" --tempo "${TEMPO:-agent}" --stroke-speed "${STROKE_SPEED:-4000}"
  status
}

down() {
  for name in firefox observatory stage displayd archied novnc wayvnc sway; do
    if running "$name"; then kill "$(cat "$RUN/$name.pid")" 2>/dev/null || true; fi
    rm -f "$RUN/$name.pid"
  done
  pg_ctlcluster 18 archie stop 2>/dev/null || true
  status
}

status() {
  for name in sway wayvnc novnc archied displayd observatory stage firefox; do
    if running "$name"; then echo "$name: running (pid $(cat "$RUN/$name.pid"))"; else echo "$name: stopped"; fi
  done
}

case "${1:-status}" in
  up) up ;;
  down) down ;;
  status) status ;;
  *) echo "usage: archie-session.sh up|down|status" >&2; exit 64 ;;
esac
