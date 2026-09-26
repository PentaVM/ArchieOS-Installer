#!/usr/bin/env bash
# Archie Arena's reset for Photopea: close the session's Firefox and open Photopea again on a
# new white document, so every run starts from the same empty canvas. The arena runs this
# before its clock starts; nothing here is Archie's doing.
#
#   arena-reset-photopea.sh [width height]   the new document's size (default 768 1144)
set -euo pipefail
RUN=${RUN:-/tmp/archie-session}
WIDTH=${1:-768}
HEIGHT=${2:-1144}
if [ -f "$RUN/firefox.pid" ] && kill -0 "$(cat "$RUN/firefox.pid")" 2>/dev/null; then
  kill "$(cat "$RUN/firefox.pid")" 2>/dev/null || true
  for _ in $(seq 1 50); do kill -0 "$(cat "$RUN/firefox.pid")" 2>/dev/null || break; sleep 0.2; done
fi
# A new profile every run: Photopea keeps its last documents and settings (a layer's
# opacity, the colours) in the browser's storage, and reopened a transparent document from
# an earlier run. Every run starts from the same nothing.
rm -rf "$RUN/ff"
mkdir -p "$RUN/ff"
firefox --headless -CreateProfile "arena $RUN/ff" >/dev/null 2>&1 || true
# One window, one tab, nothing restored.
cat > "$RUN/ff/user.js" <<'EOF'
user_pref("browser.sessionstore.resume_from_crash", false);
user_pref("browser.startup.page", 0);
user_pref("browser.tabs.warnOnClose", false);
user_pref("media.webspeech.synth.enabled", false);
EOF
rm -f "$RUN/ff/sessionstore.jsonlz4" "$RUN/ff/sessionstore-backups/"* 2>/dev/null || true
URL="https://www.photopea.com/#%7B%22script%22%3A%22app.documents.add(${WIDTH}%2C${HEIGHT}%2C72%2C%27Archie%27%2CNewDocumentMode.RGB%2CDocumentFill.WHITE)%22%7D"
( echo 900 > /proc/self/oom_score_adj
  XDG_RUNTIME_DIR=$RUN/xdg WAYLAND_DISPLAY=archie-stage MOZ_ENABLE_WAYLAND=1 LIBGL_ALWAYS_SOFTWARE=1 \
    exec nice -n 5 firefox --profile "$RUN/ff" --new-window "$URL" ) > "$RUN/firefox.log" 2>&1 &
echo $! > "$RUN/firefox.pid"
