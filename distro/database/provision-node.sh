#!/usr/bin/env bash
set -euo pipefail
umask 077
[[ $EUID == 0 ]] || exit 77
exec 9>/run/archie-database-provision.lock
flock -n 9 || exit 75
config=/etc/archie/database.json
# Existing configuration is authoritative. Never rotate credentials at boot.
if [[ -f "$config" ]]; then
  exec runuser -u archie -- /usr/bin/archie-db check "$config"
fi
admin=(runuser -u postgres -- psql -X -h /run/archie-postgres -p 55432 -d postgres -v ON_ERROR_STOP=1)
existing=$("${admin[@]}" -Atc "SELECT count(*) FROM pg_roles WHERE rolname='archie'")
[[ "$existing" == 0 ]] || { echo 'Existing database role requires operator configuration; refusing replacement.' >&2; exit 78; }
password=$(od -An -N32 -tx1 /dev/urandom | tr -d ' \n')
[[ "$password" =~ ^[a-f0-9]{64}$ ]] || exit 70
install -d -m 0750 -o root -g archie /etc/archie
# Preserve the pending credential on failure so an operator can recover. Never
# truncate an existing pending file, and never put the password in process argv.
set -o noclobber
printf '{"connection":"host=127.0.0.1 port=55432 user=archie password=%s dbname=archieos","schema":"archie_node"}\n' "$password" >"$config.pending"
chmod 0640 "$config.pending"
chown root:archie "$config.pending"
"${admin[@]}" >/dev/null <<SQL
CREATE ROLE archie LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD '$password';
CREATE DATABASE archieos OWNER archie TEMPLATE template0;
REVOKE ALL ON DATABASE archieos FROM PUBLIC;
SQL
unset password
runuser -u postgres -- psql -X -h /run/archie-postgres -p 55432 -d archieos -v ON_ERROR_STOP=1 >/dev/null <<'SQL'
CREATE EXTENSION timescaledb VERSION '2.29.2';
CREATE EXTENSION vector VERSION '0.8.6';
REVOKE CREATE ON SCHEMA public FROM PUBLIC;
SQL
runuser -u archie -- /usr/bin/archie-db check "$config.pending"
mv -T --no-clobber "$config.pending" "$config"
install -d -m 0750 -o root -g postgres /etc/archie-database
printf '{"connection":"host=/run/archie-postgres port=55432 user=postgres dbname=archieos","schema":"archie_node"}\n' >/etc/archie-database/operator.json
chmod 0640 /etc/archie-database/operator.json
chown root:postgres /etc/archie-database/operator.json
install -d -m 0700 -o postgres -g postgres /var/lib/archie-postgres-snapshots
