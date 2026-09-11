# RFC-002 — Persistent local node identity and self topology

Status: accepted for ArchieOS v0.1 (M2)

## Context

An Archie node must remain the same authority after daemon and machine restarts. Its
hardware may change, but a restart must not create a new peer identity, overwrite a
corrupt key, or discard the prior boot record.

## Decision

- The Ed25519 private key under `/var/lib/archie/identity/node.key` is the durable node
  identity. `NodeId` is derived from its public key; no independent random UUID is used.
- A missing key is created once. A corrupt key is fatal and is never regenerated
  silently.
- SQLite state under `/var/lib/archie/archie.db` records each boot and the capability
  delta from the previous boot.
- Every boot probes actual local hardware, produces a self-consistent descriptor, and
  inserts the local node into the resource graph before reporting READY.
- A node is usable without peers. The safe unconfigured profile is `dev-vm`, which may
  contribute compute but may not hold the only canonical copy of state.

## Verification contract

Boot the runtime twice against the same root and assert identical node/public identity,
two durable boot records, a real hardware descriptor with `compute.cpu`, one local node
in topology, and no fabricated remote peers.
