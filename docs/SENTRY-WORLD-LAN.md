# Sentry — LAN devices in the world graph

Step 2 of `docs/PENDIENTES.md` §3: the unified device view
(`archie sentry-devices`) lands in the world model, and the Observatory draws
the real network's devices from the graph — not from a file it read itself.

## The flow

```text
archie-lan ─┐
archie-netwatch ─┼─▶ archie-core::unify ──▶ archie sentry-devices --json
tshark import ──┘                            (archie-sentry-devices-v1 file)
                                                    │
                                                    ▼  read at startup and every poll tick
                              archie-worldd --lan-view <path>
                                    lan::LanGraph (bounded, validated, atomic)
                                                    │  GET /v1/world
                                                    ▼
                              archie-displayd (loopback client, contract-checked)
                                                    │  GET /api/world
                                                    ▼
                                            the Observatory
```

- **Ingestion** (`archie-worldd::lan`): the view document is re-validated as
  plain data against the same bounds its writer enforces — this crate has no
  dependency on `archie-core`, deliberately. Unknown fields, unknown source
  names, malformed identities, out-of-order seen times, duplicate identities
  and anything over the caps are rejected; a rejection is atomic and the
  previous graph stays. Entities are keyed by identity (hardware address, or
  the visibly weaker `ip:<address>` form), so re-ingesting the same view
  changes nothing but knowledge time.
- **Epistemics**: every entity is `LanEntityOrigin::Measurement`. The enum has
  no hypothesis variant, so this path cannot write one (RFC-008 invariant 4
  holds by construction). Event time (`first_seen_unix`/`last_seen_unix`, per
  source) and knowledge time (`ingested_at`) are separate fields (RFC-008 §4).
- **Serving**: `WorldSnapshot` carries `lan_devices` alongside the provider
  observations; the two flows never touch each other's half of the snapshot.
  `archie-displayd` validates the LAN section of the contract (count and field
  bounds) and exposes `WorldService::lan_devices()`. An oversized or malformed
  LAN section fails the whole contract — the display degrades honestly, never
  renders a partial graph.

## What a live render still requires

Everything above is proven by tests (`cargo test -p archie-worldd
-p archie-displayd`), including the view→graph→display path over a real
loopback socket. What is *not* yet built:

1. **A producer that refreshes the file.** `archie-worldd --lan-view` re-reads
   the view on every poll tick, but something must run
   `archie sentry-devices --json > <path>` on a cadence (a timer or a wrapper
   script). That scheduling is deployment, not code in this repo yet.
2. **Renderer-side drawing.** The data reaches `GET /api/world`; the Qt/QML
   observatory UI does not yet have a panel or globe layer that consumes
   `lan_devices`. Devices have no geographic position — they are LAN entities,
   so a list/panel view is the honest first render, not a globe dot.
3. **A live Windows/Linux run.** On this Windows worktree the daemons were
   exercised only through their in-process tests; a live render needs
   `archie-worldd` and `archie-displayd` running with `--lan-view` pointing at
   a fresh view file.

## Caps (mirrored from the view's writer)

| Bound | Value |
|---|---|
| View document | 8 MiB |
| Devices per graph | 4096 |
| Names / classes per device | 32 each |
| Addresses per device | 64 |
| Claims per source per device | 64 |
| Field length | 256 chars |
| worldd→displayd snapshot | 8 MiB |
