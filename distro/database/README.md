# PostgreSQL backend and verified logical snapshots

Core and the engineering dashboard use PostgreSQL exclusively. `rusqlite`, the
embedded state implementation and `node:sqlite` have been removed from their
runtime dependencies. The reviewed stack is PostgreSQL 18, TimescaleDB 2.29.2 and
pgvector 0.8.6; startup rejects missing or unreviewed extension versions. Updating
those versions requires a compatibility review, not a silent boot-time upgrade.

## Node startup

`/etc/archie/database.json` is private operator configuration:

```json
{"connection":"host=127.0.0.1 port=55432 user=archie dbname=archieos password=<generated-secret>","schema":"archie_node"}
```

The daemon must be able to read it, but cannot change it. Use root:archie 0640
and a root:archie 0750 parent on Linux. `StatePaths::rooted` locates this file at
`<root>/etc/database.json`. Absence, failed authentication, unsupported schema or
an unavailable database fails startup; there is no fallback to an empty store.
Identity keys and derived IPC exports remain files, not alternative databases.

The ISO includes the official Arch `postgresql`, `timescaledb` and `pgvector`
packages. Extension packages are pinned in `arch-packages.lock`: the build reuses
the pacman cache or downloads from the Arch Linux Archive, verifies SHA-256 and
the original Arch package signatures, then stages a build-only local repository.
The installed pacman configuration holds these two extensions until the reviewed
compatibility contract and lock are updated together. Rolling repository updates
must not produce an image whose database is rejected on first boot.
`archie-postgres-init.service` initializes only the dedicated
`/var/lib/archie-postgres` cluster, as postgres. Unknown/nonempty data directories
and other PostgreSQL majors are refused. `archie-postgres.service` binds loopback
port 55432 and a private Unix socket, with durable WAL/fsync enabled.
`archie-database.service` creates the application database, extensions and a
NOSUPERUSER/NOCREATEDB/NOCREATEROLE account. It writes a generated credential
through a private pending file and verifies the application connection before
publishing the configuration. Existing roles/configurations are never replaced.
An interrupted provisioning attempt retains its pending file for operator recovery.
`archied.service` requires successful database provisioning before it starts.
These packaging relationships are tested; Linux boot certification still needs CI.

## Metrics and vectors

`metric_samples` is an actual Timescale hypertable with one-day integer timestamp
chunks, immutable event identity and provenance. `record_metric` rejects nonfinite
values and conflicting retries; `metric_mean` limits the queried time interval.
`memory_vectors` preserves embedding versions, model/dimension identities,
provenance and sensitivity. Nearest-neighbor queries select the latest object
version at the requested time before filtering explicit permitted labels, so an
older public embedding cannot reveal a now-private object. These are storage and
retrieval primitives, not a trained embedding model or an authorization engine.

## Snapshots and restoration

The native `archie-db` binary has three operator commands:

```text
archie-db check <private-database.json>
archie-db snapshot <private-database.json> <new-snapshot-directory>
archie-db restore-verify <operator-database.json> <snapshot-directory>
```

Capture requires a dedicated application database (not postgres/template databases).
It exports one repeatable-read MVCC snapshot, streams table fingerprints and invokes
`pg_dump --format=custom --snapshot` for the complete database, including Timescale
catalogs. `manifest.json` records versions, per-table SHA-256/counts and the archive
hash/size. The source snapshot catalog records the manifest after the captured
transaction, so a new catalog entry does not invalidate the prior snapshot.
No output directory or existing file is overwritten. Private outputs are 0700/0600
on Unix; Windows uses the parent directory ACL. The current operator bound is
512 MiB, 128 tables and one million rows per table, with 120-second statement and
subprocess limits and a 128 MiB disk reserve beyond the archive bound.

Restore validates the archive checksum before creating a fresh generated database.
It creates the exact reviewed extensions, enters `timescaledb_pre_restore`, runs
single-process `pg_restore`, calls `timescaledb_post_restore` even on restore errors,
and compares the restored table fingerprints and extension versions. Only success
writes a complete `verification.json` receipt. An absent/empty receipt is not a
verified backup. Failures retain the isolated target for diagnosis and preserve
the running source. Successful restores also remain separate; promotion is an
operator action, not an automatic replacement of the running node.

On the ISO, the PostgreSQL OS account uses peer authentication with the private
`/etc/archie-database/operator.json` configuration. For example, as root:

```sh
runuser -u postgres -- archie-db snapshot /etc/archie-database/operator.json /var/lib/archie-postgres-snapshots/snapshot-001
runuser -u postgres -- archie-db restore-verify /etc/archie-database/operator.json /var/lib/archie-postgres-snapshots/snapshot-001
```

Use trusted operator-created archives only: pg_restore executes SQL; a checksum
detects damage, not a malicious archive. Snapshots contain private data, so never
send them to the engineering dashboard or commit them. This logical backup covers
the database, not identity keys, a filesystem image, PITR or crash certification.
The implementation follows the [Timescale logical backup procedure](https://github.com/timescale/Tiger-Data-Docs/blob/main/src/content/docs/deploy/self-hosted/backup-and-restore/logical-backup.mdx).

## Local verification and development

Windows extension copies are staged by `stage-test-extensions.ps1`, using the
reviewed official archive checksum and pgvector tag. No system installation is
modified. Set `ARCHIE_POSTGRES_BIN` and `ARCHIE_POSTGRES_EXTENSION_ROOT`, then run
from `tools/engineering-dashboard`:

```text
node tests/with-postgres.mjs --core
node tests/with-postgres.mjs --cognitive
node tests/with-postgres.mjs
```

Core tests use a restricted application role; snapshot tests receive a separate
operator connection. The harness creates its own SCRAM-authenticated cluster and
stops it afterward. Diagnostic files are retained. Linux CI uses
`bash distro/database/test-postgres.sh` with official distro packages.

After building `archied` and `archie-db`, `node scripts/local-node.mjs start`
provisions a separate persistent developer node under ignored
`.dashboard-local/local-node`, checks it with its restricted application account,
and runs `archied --check`. `check` repeats that boot against the same identity and
state; `stop` stops only this cluster and preserves its data. It does not install a
Windows service or touch any existing cluster. Native runtime/ISO code does not
require Node.js.

No existing application SQLite database was found in the inspected local runtime
locations. This change switches the backend and provisions fresh PostgreSQL state;
it does not claim to have converted an unseen installed database. Preserve any
older installed node/database until a source-specific validated import is complete.
Remote Core TLS, automatic restored-database promotion and point-in-time recovery
remain separate work. Vercel still needs its remote status database configuration.
