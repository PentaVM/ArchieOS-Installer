# Cognitive PostgreSQL store

`PostgresStore` implements `ExperienceStore` (events, replay, graphs, execution
checkpoints and correction trajectories) plus `SharedThoughtStore`. Core now uses
this backend through `archie-state`; no embedded database dependency or runtime
fallback remains. See `distro/database/README.md` for provisioning and snapshots.

`open_local(Config, schema)` explicitly provisions a dedicated `archie_*` schema
on PostgreSQL 18. It accepts literal loopback addresses only, rejects remote host
addresses, DNS names and explicit TLS requirements, and uses local TCP without TLS.
Remote connections must wait for a verified TLS transport. Keep credentials in
server-side configuration, never logs or command arguments. No connector or model
receives this database handle. Calls are synchronous and run the driver on scoped blocking threads. Client
creation and destruction also leave the async runtime before operating the driver.
The caller waits for completion; this is not an asynchronous connection pool.

The role must own its schema and have DDL permission. Schema version 1 is independent
of the control schema version. Its migration checksum normalizes line endings, schema changes
are transactional, and unknown versions/checksums fail closed. The schema denies
PUBLIC access. Extensions are exercised separately by the data compatibility
fixture; these contracts themselves need only ordinary PostgreSQL tables.

Each write holds a transaction-scoped advisory lock shared by this adapter's
writers, then commits the payload and outbox entry together with synchronous commit
enabled. Global serialization preserves sequence watermark ordering. PostgreSQL
sequences can have gaps after rollback; replay does not assume contiguous numbers.
Direct SQL writers must not bypass this protocol. No remote outbox sender is enabled.

Payload hashes, size bounds and indexed identities/times are checked on reads.
Focus revisions, historical parent existence, graph compare-and-swap and immutable
SharedThought repair rules remain enforced. Graph publication keeps strict
CAS (an ambiguous commit requires reading the current graph before retrying).
Event retries return the existing sequence; identical current thought retries are
accepted. Errors expose only SQLSTATE or fixed invariant labels, never server row
values. Connection establishment is bounded to 3 seconds; server statement, lock
and idle-transaction timeouts are 5, 2 and 10 seconds. These are not certification
of recovery from a frozen host or all network failure modes. Reconnect is explicit.

Execution rollback requires a fresh branch rooted on the active ancestry. Old
failed results remain historical and do not enter the resumed path. Path reads use
a consistent read transaction, with at most 1,024 ancestors and a five-second
checkpoint deadline (an individual statement can reach its server timeout).
Graph corrections preserve evidence identities, advance changed versions, and
atomically bind the supplied successor to the correction trajectory. Identical
receipt retries work after later graph publication; a separately published graph
alone cannot prove that a correction was committed. The trajectory contains its
complete correction record, rather than a redundant second correction table.

Run unit checks with `cargo test --manifest-path core/Cargo.toml --locked -p
archie-postgres -j 1`. Real integration tests are explicitly ignored by default.
Set `ARCHIE_POSTGRES_BIN` to an existing PostgreSQL 18 bin directory and run
`node tests/with-postgres.mjs --cognitive` from `tools/engineering-dashboard`.
The harness starts a new isolated SCRAM cluster, supplies a random credential only
to child processes, exercises the Rust tests and stops the cluster. Temporary
diagnostics remain local. Each Rust test reserves and removes only its own generated
schema. Compile first with `cargo test ... --no-run` on a resource-limited host.

If LLVM exhausts memory, compile only this test target with `cargo rustc
--manifest-path core/Cargo.toml --locked -p archie-postgres --test persistence -j 1
-- -C codegen-units=1`. Set `ARCHIE_COGNITIVE_TEST_BINARY` to the resulting test
executable for the harness; `--message-format=json` returns its exact artifact path.
This reduces test code generation concurrency without rebuilding dependencies or
changing the workspace build profile.

The reviewed driver is pinned to postgres 0.19.10; its lockfile dependencies were
resolved with `CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback` for the workspace's
Rust 1.88 contract. Use `--locked`; review MSRV before driver/lockfile updates.
