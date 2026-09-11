#!/usr/bin/env bash
# Linux/CI fixture: creates its own cluster and never touches a system service.
set -euo pipefail
umask 077
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
fixture=$(mktemp -d /tmp/archie-pg-tests.XXXXXXXX)
password=$(od -An -N32 -tx1 /dev/urandom | tr -d ' \n')
printf '%s' "$password" >"$fixture/password"
owner=()
if [[ $EUID == 0 ]]; then
  chown -R postgres:postgres "$fixture"
  owner=(runuser -u postgres --)
fi
"${owner[@]}" initdb -D "$fixture/data" --username=archie_test --auth=scram-sha-256 --pwfile="$fixture/password" --encoding=UTF8 --locale=C >/dev/null
rm -- "$fixture/password"
printf "\nshared_preload_libraries='timescaledb'\ntimescaledb.telemetry_level='off'\n" >>"$fixture/data/postgresql.conf"
started=0
finish() {
  if [[ $started == 1 ]]; then "${owner[@]}" pg_ctl -D "$fixture/data" -m fast -w stop; fi
}
trap finish EXIT
"${owner[@]}" pg_ctl -D "$fixture/data" -l "$fixture/server.log" -o "-h 127.0.0.1 -p 55439 -k $fixture -c shared_buffers=32MB" -w start
started=1
export PGPASSWORD="$password"
psql -X -h 127.0.0.1 -p 55439 -U archie_test -d postgres -v ON_ERROR_STOP=1 -c "CREATE EXTENSION timescaledb VERSION '2.29.2'; CREATE EXTENSION vector VERSION '0.8.6'"
export ARCHIE_TEST_DATABASE_URL="postgresql://archie_test:$password@127.0.0.1:55439/postgres"
export ARCHIE_COGNITIVE_TEST_DATABASE_URL="$ARCHIE_TEST_DATABASE_URL"
export ARCHIE_TEST_ADMIN_DATABASE_URL="$ARCHIE_TEST_DATABASE_URL"
unset password PGPASSWORD
cargo test --manifest-path "$root/core/Cargo.toml" --workspace --locked -- --test-threads=1
cargo test --manifest-path "$root/core/Cargo.toml" --locked -p archie-postgres --test persistence -- --ignored --test-threads=1
