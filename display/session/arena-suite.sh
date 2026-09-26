#!/usr/bin/env bash
# Archie Arena's suite: the ArchieOS session up, every task at every tier it names, then the
# session down again — whatever happens — so that between benchmarks only training runs.
# One memory per application (photopea.json): what Archie learns about the editor carries
# from one painting to the next.
#
#   arena-suite.sh [task.toml ...]   (default: every task in core/crates/archie-arena/tasks)
#   TIERS="15 60" arena-suite.sh     (default: each task's own tiers)
set -euo pipefail
REPO=${REPO:-/workspace/archieos}
ARENA=${ARENA:-/workspace/arena}
RUN=${RUN:-/tmp/archie-session}
mkdir -p "$ARENA/memory"
exec 9>"$ARENA/suite.lock"
flock -n 9 || { echo "arena-suite: already running" >&2; exit 0; }
echo "$$" > "$ARENA/suite.running"
cleanup() {
  rm -f "$ARENA/suite.running"
  "$REPO/display/session/archie-session.sh" down >/dev/null 2>&1 || true
}
trap cleanup EXIT
set -a
. /workspace/archie-live.env
set +a
"$REPO/display/session/archie-session.sh" up >/dev/null
for _ in $(seq 1 60); do [ -S "$RUN/control.sock" ] && break; sleep 1; done
cd "$REPO"
TASKS=("$@")
[ ${#TASKS[@]} -gt 0 ] || TASKS=(core/crates/archie-arena/tasks/*.toml)
for task in "${TASKS[@]}"; do
  tiers=${TIERS:-$(sed -n 's/^tiers = \[\(.*\)\]/\1/p' "$task" | tr ',' ' ')}
  for tier in $tiers; do
    echo "{\"event\": \"arena_run\", \"task\": \"$task\", \"tier\": $tier}"
    nice -n 5 ./core/target/release/archie-arena run --task "$task" --tier "$tier" \
      --socket "$RUN/control.sock" --key-file "$RUN/key" --policy display/session/arena-apps.toml \
      --out "$ARENA" --memory "$ARENA/memory/photopea.json" 2>&1 | tail -1 | cut -c1-400 || true
  done
done
