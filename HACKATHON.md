# Nebius x NVIDIA Global AI Hackathon

## Provenance

The migration baseline is `PentaVM/ArchieOS` commit `045e782` (11 September 2026),
recorded locally in the source repository as annotated tag
`pre-nvidia-migration-2026-09-11`. The source repository and its remote branches were
not rewritten or deleted.

Pre-existing ArchieOS work includes the Rust control plane, PostgreSQL/TimescaleDB and
pgvector state, capability-aware routing, policy and execution boundaries, native Qt
observatory, GeoFabric foundations, OSM building ingestion, C++ Analytical Cortex,
Sentry, capture agents, installer, and ArchISO packaging.

Four components that a reader might reasonably assume were built for this hackathon were
not, and are named here so that nothing in this repository overstates what is new:

- **The learning admission firewall** (`archie-memory::admission`, RFC-011) and
  **bounded analytical synthesis** (`archie-memory::analysis`, RFC-012), which decide
  what may enter memory at all and keep perception from accumulating noise.
- **The conversational floor manager** (`archie-memory::floor`), which decides whether
  Archie should speak from speaker-hold, thought-completion and interruption-utility
  estimates, and defaults to silence.
- **Wanda** (`sentry/identity/wanda`), deterministic social inference that produces
  hypotheses a subject must declare before they become relationships.
- **Privacy as a routing disqualifier** rather than a weighted score, in `archie-router`.

## Work in this repository

- Extracted the active runtime into an independent history and excluded the retired
  Python/PySide6 application, stale release records, caches, and generated output.
- Added explicit NVIDIA NIM and Nebius Token Factory provider kinds without coupling
  cognitive roles to either provider.
- Added an OpenAI-compatible inference transport with endpoint validation, bounded
  responses, environment-only credentials, timeout handling, and a second privacy gate
  immediately before network egress.
- Added a small CLI smoke path for model discovery, readiness, and chat requests.
- Added an optional NVIDIA package profile and staged provider manifests in the ISO.
- Replaced legacy CI with checks that represent the migrated Rust and C++ stack.

### The privacy spine

- Added `PrivacyGrade::Secret`, a grade no locality permits. The four existing grades
  answer *where* data may be processed; this one answers *whether*, and the answer is no
  — a password contributes nothing to reasoning and its disclosure cannot be withdrawn.
- Added `DataRetention` with an `Unknown` default treated exactly as `Retains`. A
  provider advertising zero data retention and this deployment having verified it are
  different facts, and only the second is a safety property. Non-public data to an
  unverified third party is now disqualified at routing.
- Added `archie-vault`: AES-256-GCM sealed entries (via `ring`, already in the graph
  through rustls) whose grade is authenticated rather than labelled, so rewriting a
  stored record's grade yields a decryption failure instead of a readable entry.
  Credentials leave only through a broker whose handle cannot be cloned, serialised or
  printed.
- Added the context gateway: task-bound, TTL-limited capsules that pseudonymise entities
  when the destination is a third party, resolve the aliases locally afterwards, and are
  deliberately not serialisable, so private context cannot become a trace by accident.
- Added `archie-mcp`: Model Context Protocol entered through the capability model.
  `AuthorizedCall` has no public constructor, so the path from a model's proposed tool
  call to a transport necessarily passes the gate. The operator's allowlist is the
  authority; `tools/list` is the server describing itself.
- Added the streamable HTTP transport for it, covering `initialize` negotiation,
  session binding, `tools/list`, `tools/call`, `resources/list` and `resources/read`,
  both the JSON and event-stream response shapes, per-call credential brokering, and
  response budgets enforced before allocation. The `stdio` transport is not implemented,
  and no stub resembling one ships.

Vault references (`vault:...`) existed throughout the pre-existing codebase; what was
missing was anything that resolved, encrypted or enforced them. That half is the new
work here.

No benchmark numbers are claimed here. Test results and any demo measurements must come
from executed builds on identified hardware.

### Web search

- Added `archie-search`: web search through Tavily, entered through the capability model.
  A query is a disclosure, so it passes a gate the way a tool call does.
  `AuthorizedSearch` has no public constructor, a caller must hold `network.internet`,
  and the query's grade is checked against the provider's validated clearance. A cloud
  provider whose retention nobody has verified is cleared for public queries only;
  a manifest that claims more fails validation instead of taking effect.
- The client uses HTTPS only, follows no redirects, and enforces its response budget
  before allocating. It sends the key as a bearer header brokered from the vault per
  call. Tavily's generated answer, raw page content, images, country and automatic
  parameters are never requested, and each is sent as `false`.
- Results are bounded, stripped of control and bidirectional-override characters, kept
  only for ordinary `http`/`https` pages, and given a `web:` reference that a later stage
  can cite without quoting the page. Refusals, errors, receipts and debug output never
  contain the query, and provider error text has echoed keys and queries blanked out.
- `Cerebra::propose_search` records the search at `SelectCapability` with its grade and
  never its words. `archie-search` is a CLI smoke path, like `archie-infer`, that reads
  `TAVILY_API_KEY`. `providers/search/tavily.toml` ships public-only and unverified.
- Verified against loopback listeners, not the live API. Eleven rules were broken on
  purpose, and a test failed for each. One of them first survived: the only
  credentials-in-URL case carried both a username and a password, so either check hid
  the other. The test was fixed rather than recorded as passing. `extract`, `crawl` and
  credit accounting are not implemented.

### Cerebra runtime additions

- Added a bounded advisory interpreter as an optional continuation after Cerebra's
  existing floor/context/router sequence. It uses real OpenAI-compatible HTTP and
  gateway capsules, with synthetic loopback tests and content-free refusal traces.
- The analytical gate and floor policy predate this work. The existing gate abstains
  throughout the ambiguous band; this work does not change it or claim a measured
  analytical improvement. Live provider and labelled-corpus evaluation remain pending.
- Added an integer context-utility learner, digest-bound to admitted outcomes, with
  exposure/risk costs, requested-reference-only gateway ranking and encrypted local
  parameter checkpoints. Tests compare explicitly synthetic contexts; no production
  training corpus, promotion or accuracy is claimed. The admission firewall is reused
  unchanged, including its eight-field payload-free curated record.
- Added a local floor baseline with bounded ephemeral audio features, online updates
  that keep only integer weights, and encrypted parameter checkpoints. This is new;
  `decide_floor` remains the pre-existing deterministic decision. No real labelled floor
  dataset, microphone driver integration, retained recordings or accuracy claim ships.
- Disabled inference HTTP redirects so an endpoint cannot forward admitted context
  beyond the destination selected by routing; verified with two loopback listeners.
