#!/usr/bin/env bash
set -euo pipefail
umask 077
data=/var/lib/archie-postgres
[[ $(id -un) == postgres ]] || exit 77
if [[ -f "$data/PG_VERSION" ]]; then
  [[ $(cat "$data/PG_VERSION") == 18 ]] || exit 78
  exit 0
fi
# Refuse an unknown or interrupted cluster. Never reinitialize or remove data.
[[ -z $(find "$data" -mindepth 1 -maxdepth 1 -print -quit) ]] || exit 78
initdb -D "$data" --username=postgres --auth-local=peer --auth-host=scram-sha-256 --encoding=UTF8 --locale=C >/dev/null
cat >>"$data/postgresql.conf" <<'CONFIG'
listen_addresses = '127.0.0.1'
port = 55432
unix_socket_directories = '/run/archie-postgres'
unix_socket_permissions = 0700
shared_preload_libraries = 'timescaledb'
shared_buffers = '64MB'
max_connections = 32
password_encryption = 'scram-sha-256'
synchronous_commit = on
fsync = on
full_page_writes = on
timescaledb.telemetry_level = 'off'
CONFIG
cat >"$data/pg_hba.conf" <<'HBA'
local all postgres peer
local all all reject
host archieos archie 127.0.0.1/32 scram-sha-256
host all all 0.0.0.0/0 reject
host all all ::/0 reject
HBA
