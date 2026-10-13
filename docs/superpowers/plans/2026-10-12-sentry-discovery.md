# Sentry LAN Discovery Implementation Plan

> Execution: superpowers:executing-plans, directly in the existing Nvidia worktree.

**Goal:** Expand real device discovery without hardcoding local devices.
**Architecture:** Parallel bounded observation sources feed the existing inventory;
diagnostics describe incomplete coverage independently of that strict contract.
**Tech stack:** Rust, socket2, roxmltree, if-addrs, existing serde/uuid.
**Spec:** ../specs/2026-10-12-sentry-discovery-design.md

## Global constraints

Only ArchieOS-Nvidia. Preserve Core policy/leases and inventory v1.
Maximum 256 subnet addresses, 16 TCP workers, 250 ms per connect, default 15 seconds
total with an explicit configurable deadline up to 120 seconds.
No application payload in TCP presence probes. No inferred owner from Spotify.

## Review focus

- UDP advertisements can include poisoned endpoints: require same observed host.
- Multicast bind can fail: retain unicast and report degraded coverage.
- One source can fail: preserve other successful results and expose the error.
- Virtual/large/public subnets: reject automatic expansion beyond the declared cap.
- Open ports and generic automation services: keep unknown identity/class honest.

## Tasks

1. mDNS (`mdns.rs`, `inventory.rs`, Cargo files): test missing direct query discovery,
   multicast reception, same-instance printer SRV/TXT and hostile paths first; run
   failing tests, then implement bounded multicast/unicast browsing and service facts.
2. WS-Discovery (`wsdiscovery.rs`, `inventory.rs`): test both namespace dialects,
   request correlation, malformed XML, printer/camera QNames and foreign endpoints;
   implement read-only probes and merge evidence without rewriting existing facts.
3. TCP presence (`presence.rs`): test CIDR cap/public rejection, interface subnet
   selection, actual accepting TCP listener and unknown classification; implement
   bounded connection-only probes and open-port claims.
4. Integration (`main.rs`, `inventory.rs`): test malformed options, partial discovery
   failure, schema compatibility and diagnostics; implement --bind, --probe-local,
   --subnet and --diagnostics options for scan and watch.
5. Verification/documentation: affected crate tests, Core feed/unify compatibility,
   formatting/clippy, rebuilt live black-box comparison and local evidence; update
   SENTRY-LAN and _dev ledger. Commit and publish a focused PR with verified limits.

Live-evidence rulings: add port-first scheduling and configurable deadline after
the first 15-second run reached only 65 of 253 hosts; add Sentry-owned ARP cache
observation for silent candidates; add Spotify getInfo for declared type evidence.
Review regressions: assigned-interface validation, cache-only freshness in Core/feed,
companion-link ambiguity, actual multicast reception and independent-source recovery.

Each implementation task follows RED → GREEN and a relevant suite run. Execute all
tasks inline under the user's existing authorization; no additional design gates.
