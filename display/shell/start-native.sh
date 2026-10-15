#!/usr/bin/env bash
set -euo pipefail
# Same native renderer and OS authority boundary on the DRM seat or a WSLg session.
display_bin="${ARCHIE_DISPLAY_BIN:-/usr/bin/archie-display}"
bridge_bin="${ARCHIE_SHELL_BRIDGE_BIN:-/usr/bin/archie-shell-bridge}"
[[ -x "$display_bin" && -x "$bridge_bin" ]] || {
  printf '%s\n' 'Native display and read-only shell bridge must both be installed.' >&2
  exit 69
}
if [[ -n "${WAYLAND_DISPLAY:-}" ]]; then
  export QT_QPA_PLATFORM="${QT_QPA_PLATFORM:-wayland}"
fi
export QT_QUICK_CONTROLS_STYLE=Basic
exec "$display_bin" --shell --shell-bridge "$bridge_bin" "$@"
