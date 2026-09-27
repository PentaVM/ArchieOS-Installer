#!/usr/bin/env bash
# Archie Arena's reset for car-soccer.com: close the session's Firefox and open the game again
# with a fresh profile, so every run starts in the same free-play arena. It loads 216 files and
# warms its effects first; the live runner waits (--settle-s) before anything is timed. Nothing
# here is Archie's doing.
set -euo pipefail
RUN=${RUN:-/tmp/archie-session}
if [ -f "$RUN/firefox.pid" ] && kill -0 "$(cat "$RUN/firefox.pid")" 2>/dev/null; then
  kill "$(cat "$RUN/firefox.pid")" 2>/dev/null || true
  for _ in $(seq 1 50); do kill -0 "$(cat "$RUN/firefox.pid")" 2>/dev/null || break; sleep 0.2; done
fi
rm -rf "$RUN/ff"
mkdir -p "$RUN/ff"
firefox --headless -CreateProfile "arena $RUN/ff" >/dev/null 2>&1 || true
cat > "$RUN/ff/user.js" <<'P'
user_pref("browser.sessionstore.resume_from_crash", false);
user_pref("browser.startup.page", 0);
user_pref("browser.tabs.warnOnClose", false);
user_pref("media.autoplay.default", 5);
P
( echo 900 > /proc/self/oom_score_adj
  XDG_RUNTIME_DIR=$RUN/xdg WAYLAND_DISPLAY=archie-stage MOZ_ENABLE_WAYLAND=1 LIBGL_ALWAYS_SOFTWARE=1 \
    exec nice -n 5 firefox --profile "$RUN/ff" --new-window "https://car-soccer.com/" ) > "$RUN/firefox.log" 2>&1 &
echo $! > "$RUN/firefox.pid"
