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

### The Archie Stage

- Added the Archie Stage (`display/stage`, [docs/STAGE.md](docs/STAGE.md)), a Wayland
  compositor in which Archie operates applications with a visible pointer and keystrokes,
  with nobody's mouse needed and nobody's mouse locked out. Any physical input takes the
  stage back at once. The agent resumes after eight quiet seconds, and a pause chord holds
  it until a person resumes it. The agent's own input goes through the seat and cannot
  trigger either.
- An action takes one path, whoever asks: `Cerebra::propose_stage_action` or
  `archie_stage::Operator`, then the stage policy, a one-operation lease, a gate whose
  output is the only thing that can be sealed, an HMAC-sealed line, and the compositor's
  own re-check against the window's geometry revision.
- Applications get text the way they get a person's: keys found on the keymap they were
  given, with Shift, AltGr and modifier state moved as a physical key moves them. A text
  with characters off the keymap goes whole through the application's input method. With
  neither route, the receipt says `not_typeable` instead of `executed`. Qt's public seat
  API sends Ctrl+C as a plain "c" and drops text for applications without text-input, so
  this uses the private calls Qt's own keyboard handling uses.
- Cinematics for any application come from a pure, display-free model (`director.hpp`).
  The camera frames the action and does not chase the cursor, and cuts to the whole stage
  on a takeover. Pointer paths are Fitts-timed, and ripples are bounded. Captions and a
  strip of recent actions carry verbs, applications and counts, never typed text. Frames
  are requested only while something moves: 609 in a scripted 16 s session, against 960
  at 60 fps.
- Verified in WSLg against real clients, reading their Wayland protocol traces:
  - A demo form saved "Archie escribió esto solo" with no human input.
  - A Ctrl+A chord replaced a draft, and "Hola, ñandú: 100% listo!" arrived in order.
  - `weston-terminal` ran a typed shell command with quotes and redirection.
  - A mouse moved over the window during one run took the stage, and the agent resumed.
- Not working: `foot` (it needs protocol versions Qt 6.11's compositor does not offer),
  Alacritty at full size, targeting popups, XWayland, and any run on DRM/KMS hardware.

### Archie for IDE agents

- Added `archie-mcp-server`, which serves Archie's tools over MCP on stdio to VS Code's agent
  mode, Claude Code, Zed and any other MCP client. It is one Rust binary, with no IDE
  extension. The research behind the choice, including why Live Share is not viable and
  why Zed is the recommended editor for the image, is in
  [docs/research/IDE.md](docs/research/IDE.md).
- A tool result leaves the node for a model Archie cannot see, so each tool declares a
  privacy grade. The server's clearance defaults to public and cannot go above internal.
  Node status is an allowlist that leaves out the hostname, network interfaces and
  location.
- Operating stage applications is opt-in, goes through the same `Operator` and gate, runs
  as the IDE agent's own principal, and can never obtain an approval or clear a person's
  takeover.

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

### Workstation alpha additions

Built for this hackathon after the original control-plane work:

- Reproducible Arch Linux WSL rootfs and modern `.wsl` packaging, a fallback importer,
  systemd units, non-root session, disabled Windows automount/interop, and `archie`
  first-run/health commands.
- A native deterministic clarification gate in Cerebra with explicit permille inputs,
  floor-aware deferral, pre-commit/pre-external-effect states, answer-ref resumption and
  duplicate-question prevention.
- Provider discovery/config for Nebius, Cerebras, OpenRouter, Ollama and vLLM, plus a
  replaceable loopback UHP client for HarnessRouter-style Codex/Claude/Hermes workers.
- A pinned first-run build of NVIDIA NeMo-Speech.cpp and checksum-pinned Nemotron Speech
  Q8_0 weights. Audio buffers are bounded and ephemeral; partial/final text remains
  unadmitted until Cerebra decides.
- A policy-lease-gated FreeCAD process adapter and real headless parametric bracket
  smoke test using the official Arch package.

The original floor decision, learning-admission firewall, analytical gate, Vault and
capability policy remain pre-existing foundations. The workstation work composes them;
it does not relabel them as hackathon output. No speech accuracy, floor accuracy or
professional CAD fitness metric is claimed.

### IRIS perception and Foundry additions

Built for this hackathon:

- Added the `archie-iris` typed perception fabric: a revisioned scene graph, integer
  transforms, bounded ephemeral frames, damage/semantic ROI extraction, structured-first
  browser/CAD sources, explicit specialist seams, Visual Reflex/Cortex contracts, bounded
  scene memory, ACP, policy-lease motor compilation and shared-weight source sessions.
- Added a fixed-point INT8 deployment loader. `archie iris verify-model` checks file
  bounds, manifest shape, checkpoint digest and tensor bounds locally before inference.
- Created the separate `PentaVM/archie-iris` Foundry repository with license-gated
  streaming, a bounded cache, checkpoint/resume, deterministic synthetic training, sealed
  evaluation, INT8 export, champion/challenger promotion and bounded Forge experiments.
- Composed PostgreSQL 18, reviewed signed TimescaleDB/pgvector packages and `archie-db`
  into the WSL image. The control plane now waits for durable-state provisioning.
- Added a LocateAnything grounding adapter over the OpenAI-compatible vision API. It
  accepts only Vault-admitted context and a digest-bound requested ROI, returns advisory
  geometry, and was verified against a loopback server including a zero-connection
  refusal. The NVIDIA weights are not bundled and their non-commercial research license
  prevents treating them as Archie's production model.
- Exercised that boundary against the live Nebius Token Factory API with
  `google/gemma-3-27b-it`, a Public Vault capsule and a synthetic image created in memory.
  The returned proposal was mapped into Archie scene coordinates. This is one functional
  smoke test and carries no grounding-accuracy claim.

The checked-in Visual Reflex checkpoint and its 1000/1000 sealed result use only the
declared synthetic generator; they are pipeline validation, not a production accuracy
claim. Production UI/camera adapters, OCR/grounding providers, real labelled benchmarks
and Nebius cloud training remain unvalidated. No private frames, audio, transcripts or
biometric examples are retained.
