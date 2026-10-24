# Sentry autonomous monitor implementation plan

**Execution:** superpowers:executing-plans in the existing isolated Nvidia worktree.
**Goal:** Add an upstream DNS-SD observer and autonomous, recoverable LAN monitoring.
**Architecture:** Supplemental bounded service discovery, persistent history and
independent scan cadences reuse the inventory writer and existing Core/feed contract.
**Spec:** ../../SENTRY-AUTONOMY.md
**Stack:** Rust, mdns-sd 0.21.5 (MIT/Apache-2.0), existing serde/socket/interface APIs.

## Global constraints

No replacement control authority, automatic pairing, credential guessing or printing.
Keep inventory v1. Maximum 4096 history entries, 8 MiB input, 64 service kinds,
256 TCP scope addresses, 16 workers. Preserve stale/candidate provenance.

## Tasks

1. Write failing conversion/backend tests; add a bounded mdns-sd observer, scoped
   interface selection, private IPv4/IPv6 service observations and native fallback.
   Expose --mdns-backend on scan/watch without changing the legacy default.
2. Write failing restart/history/retention/backoff/cadence tests; implement the
   autonomous controller, separate light/deep scheduling, private state/health,
   --autonomous and --once. Preserve successful output when a scan fails.
3. Test history-only freshness in Core and the context feeder, then extend cache
   provenance handling. Verify strict schema and current state are not fabricated.
4. Run affected suites/lint, bounded native/hybrid/autonomous black-box comparison,
   independent review and fixes. Update PR39/bitácora/docs with actual upstream,
   runtime and hardware limitations; keep home identifiers out of GitHub.

## Review focus

Daemon bind failures after constructor success; advertised foreign addresses;
stale service caches; corrupt/oversized persisted state; unsuccessful light scans;
unsupported deep scopes; clock rollback and restart scheduling; authority boundaries.
