#!/usr/bin/env bash
# A compositor only for the live installer; the installed observatory keeps EGLFS.
set -euo pipefail
[[ -d /run/archiso ]] || { echo "Run this from the ArchieOS live USB." >&2; exit 64; }
if [[ -n "${WAYLAND_DISPLAY:-}${DISPLAY:-}" ]]; then exec archie-wizard; fi
[[ $(id -u) == 0 ]] || { echo "Start the live installer session as root." >&2; exit 77; }
export XDG_RUNTIME_DIR
XDG_RUNTIME_DIR=$(mktemp -d /run/archie-wizard.XXXXXXXX)
chmod 700 "${XDG_RUNTIME_DIR}"
systemctl stop archie-display.service
weston --backend=drm --xwayland --socket=archie-wizard --idle-time=0 >"${XDG_RUNTIME_DIR}/weston.log" 2>&1 &
archie_weston_pid=$!
cleanup() {
  kill "${archie_weston_pid}" 2>/dev/null || true
  wait "${archie_weston_pid}" 2>/dev/null || true
  systemctl start archie-display.service || true
}
trap cleanup EXIT
for attempt in $(seq 1 100); do
  [[ -S "${XDG_RUNTIME_DIR}/archie-wizard" ]] && break
  kill -0 "${archie_weston_pid}" 2>/dev/null || { echo "Compositor failed; inspect ${XDG_RUNTIME_DIR}/weston.log" >&2; exit 1; }
  sleep 0.1
done
[[ -S "${XDG_RUNTIME_DIR}/archie-wizard" ]] || exit 1
export WAYLAND_DISPLAY=archie-wizard QT_QPA_PLATFORM=wayland
archie-wizard
