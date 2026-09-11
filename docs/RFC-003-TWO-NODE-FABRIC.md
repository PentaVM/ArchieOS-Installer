# RFC-003 — Authenticated two-node fabric vertical slice

Status: accepted for ArchieOS v0.1 (M3)

## Context

Fabric discovery must produce useful control-plane topology without turning LAN
visibility into unrestricted trust. The transport must remain replaceable and tests
must not pretend Zenoh is already implemented.

## Decision

- Nodes announce self-consistent capability descriptors in signed Ed25519 envelopes.
- Receivers verify the signature and the `NodeId`/public-key/descriptor binding before
  accepting a peer.
- First valid contact pins the key (`pinned`); trust is receiver-owned and never taken
  from a peer claim. Explicit operator promotion is required for sensitive data.
- Accepted descriptors enter both durable peer state and the live resource graph.
- `archie peers`, `archie status`, and `archie topology` read that graph through the
  existing control protocol.
- The deterministic M3 acceptance transport is the in-process `LoopbackBus`. UDP is
  implemented and tested separately. Zenoh remains future work behind `Transport`.

## Verification contract

Boot two complete runtimes on different state roots and a shared bus. Each must receive
and authenticate the other's signed announcement, persist the pinned peer, report it as
reachable through the control contract, and expose both nodes in topology.
