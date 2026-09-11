#!/usr/bin/env bash
set -euo pipefail
for ((attempt=0; attempt<60; attempt++)); do
  if pg_isready -q -h /run/archie-postgres -p 55432 -U postgres; then exit 0; fi
  sleep 1
done
exit 1
