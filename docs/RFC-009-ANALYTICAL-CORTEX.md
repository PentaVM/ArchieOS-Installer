# RFC-009: Archie Analytical Cortex

Status: canonical production architecture. Native deterministic retrieval, local persistent
graph/Focus/experience and atomic correction publication are implemented. Learned shared
temporal/heterogeneous heads are DATA_STARVED. Operator-facing correction parsing,
provider dispatch, training and promoted neural inference are not yet operational.

Execution checkpoints, storage contracts and bounded explicit prospection are also
implemented and locally tested. Full Linux/ISO validation of this branch is currently
blocked before runner startup by the repository account's Actions billing limit.

## Decision

Archie's persistent knowledge remains outside all model weights. A dedicated,
unprivileged native C++23 process, `archie-cortexd`, learns or scores how a
policy-visible temporal graph should be used to assemble context for a replaceable
generative provider. The daemon is never an authority for facts, policy, leases,
scheduler decisions or process execution.

```text
trusted sources -> raw/private vault + event log -> temporal graph snapshot
                                                   |
                                      policy-filtered, immutable view
                                                   |
archied / memory ------------------------------> archie-cortexd
                                                   |
                                          ContextBundle / proposal
                                                   |
                                      router -> replaceable model worker
```

The current compiler is an always-available deterministic baseline. A future shared
temporal graph encoder may add Coreference, EntityLink, Retrieval, GraphExpansion,
ContextBudget, KnowledgeGap, ProcessUtility and ResourcePrediction heads, but only after
replay data and benchmark evidence show an improvement over this baseline.

## Invariants

1. The graph/event/vault stores truth and provenance; weights never become the source of
   truth.
2. The Cortex returns a `ContextBundle` or `AnalyticalProposal`, never a verified fact or
   a side effect.
3. Policy filters sensitivity before context compilation. Sensitivity labels are an
   explicit allow-list, not an ordered privilege hierarchy.
4. Every selected node and edge preserves a provenance reference.
5. Context selection is bounded by an explicit token budget and deterministic tie-breaks.
6. Corrections are typed supervision and references to authoritative graph changes; the
   Cortex does not apply those changes itself. A trajectory is valid only after a trusted
   graph-writer receipt binds it to the successor snapshot.
7. Candidate neural models run in replay/shadow/canary only. Promotion and rollback live
   in the trusted model registry and require reproducible model/dataset/schema identity.
8. Archie remains functional when the Cortex, GPU, ONNX Runtime or TensorRT is absent:
   callers retain a deterministic retrieval fallback.

## Current A0/A1 implementation

`cortex/include/archie/cortex/contracts.hpp` defines bounded temporal `GraphSnapshot`,
`GraphNode`, `GraphEdge`, `CorrectionEvent`, `ModelManifest`, `ContextRequest`,
`ContextBundle` and analytical-proposal contracts. Snapshots reject duplicate ids,
dangling edges, absent provenance, invalid times, oversized input and unbounded requests.

`DeterministicContextCompiler` is L0:

- lexical relevance plus recency and provenance scoring;
- exact sensitivity filtering;
- layer-synchronous zero-to-three-hop expansion from lexical, project and Focus seeds;
- deterministic utility-per-token ordering, path closure and visible contradiction inclusion;
- source-cited rendering with the entire rendered envelope charged to the budget.
  Token counts are byte/4 estimates plus reserve; providers must enforce their exact tokenizer.

`cortex/include/archie/cortex/ipc.hpp` specifies versioned, length-prefixed binary IPC.
It contains only graph snapshots, context requests, context responses and refusals. There
is deliberately no message for commands, policy writes, leases, credentials, raw-vault
contents or graph mutation.

`archie-cortexd` reads an immutable, atomically replaced graph snapshot file and serves
only `ContextRequest -> ContextBundle` on an AF_UNIX stream. Production socket activation
permits only the UID `archie` (`archied`) after `SO_PEERCRED` validation. A direct socket
is available solely for isolated same-UID development tests.

## A2 correction-trajectory foundation

The canonical C++ contracts now distinguish `CorrectionEvent`,
`GraphCorrectionReceipt` and `CorrectionTrajectory`. They contain IDs, immutable vault
or provenance references and SHA-256 digests rather than copied voice, conversation,
model-response, location or biometric payloads. A receipt must name the same correction
and source snapshot, advance to a successor snapshot and be no older than the correction.

The trusted Rust control plane mirrors those contracts in `archie-memory` and persists
the event plus its one training trajectory in the same SQLite transaction. Stored JSON
is SHA-256 verified on every read. Writes are idempotent only when the correction and
trajectory IDs bind to byte-identical payload; conflicting reuse is rejected. This store
is deliberately internal: it adds no `archied` mutation command, parser, model promotion
or raw-vault read path. The trusted graph publication API is described below.

`protocols/cortex-correction-trajectory.schema.json` is the language-independent JSON
contract for parsers and offline tooling. A Rust agreement test keeps its correction-kind
vocabulary equal to the trusted types and rejects raw voice/message/response/biometric
fields at the trajectory boundary.

The operational A2 sequence is therefore constrained as follows:

```text
voice/text captured in private vault -> parser/confirmation (pending)
-> trusted graph writer applies factual correction (implemented)
-> validated graph-update receipt -> atomic trajectory persistence (implemented)
-> offline dataset/replay job (pending)
```

## Neural runtime and promotion path

The source tree has a CMake option for ONNX Runtime, disabled by default. It adds no
runtime dependency or model artifact to the current image. TensorRT/CUDA is an optional
future backend for a promoted, measured ONNX export; it is not assumed merely because an
RTX card exists.

Each non-baseline `ModelManifest` requires a model hash, training dataset version, graph
schema version, architecture, promoted time and rollback model. An unpromoted candidate
is explicitly refused by the production contract. The required sequence is:

```text
correction/trajectory -> versioned dataset -> candidate checkpoint -> replay metrics
-> shadow ContextBundle comparison -> canary -> explicit promotion or rollback
```

No live weight update follows an individual message. There is no model checkpoint,
training dataset, ONNX artifact, TensorRT engine or measured GPU benchmark in the current
repository, so none is claimed as shipped.

## Runtime filesystem and sandbox

| Path | Owner/access | Purpose |
|---|---|---|
| `/etc/archie/cortex/` | `root:archie-cortex`, `0750` | operator-owned Cortex configuration |
| `/var/lib/archie/cortex/` | `archie-cortex`, `0750` | derived cache and future model registry state |
| `/var/lib/archie/knowledge/graph/` | `archie:archie-cortex`, `2750` | trusted immutable graph snapshots, read-only to Cortex |
| `/var/lib/archie/private/` | `root:root`, `0700` | raw private vault; inaccessible to Cortex baseline |
| `/var/cache/archie/cortex/` | `archie-cortex`, `0750` | disposable graph/model cache |
| `/run/archie/cortex.sock` | socket activation, group `archie`, `0660` plus UID check | local typed IPC only |

`archie-cortex.service` runs as `archie-cortex` with no Linux capabilities, a 30% CPU
quota, 384 MiB memory cap, no network namespace, AF_UNIX only, protected system/home and
read-only knowledge input. It cannot read the raw private vault, access the control
socket, call the network, execute a worker or modify the graph source.

## Failure posture

| Condition | Required behavior |
|---|---|
| Cortex unavailable | caller uses deterministic non-Cortex retrieval; no authority changes |
| malformed/oversized IPC or snapshot | reject bounded input; never parse partially |
| policy-visible snapshot absent | typed refusal, no invented context |
| model artifact corrupted/unpromoted | retain previous promoted artifact or deterministic baseline |
| GPU/TensorRT absent | ONNX/CPU only if a validated model is enabled; otherwise baseline |
| trainer crash | separate offline job; production selection unaffected |

## Acceptance evidence and remaining work

The portable C++ contracts have unit coverage for graph validation, provenance/sensitivity
gates, bounded expansion, deterministic budget selection, correction/model promotion
boundaries and binary IPC round trips. A Linux-only daemon E2E test creates an immutable
snapshot, launches `archie-cortexd`, performs an AF_UNIX request and validates the
returned bundle. Arch CI runs the CMake/CTest suite and the ISO build repeats it before
staging the binary.

Still required before a complete persistent brain claim: encrypted raw vault,
retention/delete/export, worker dispatch, correction parser, real quality datasets,
neural training/export, shadow/canary promotion, and measured RTX/Qwen co-residency.
These are canonical architecture with explicit maturity gates, not a separate
experimental feature area.

## Local cognitive substrate, baseline v2

The existing `archie-state` SQLite backend now stores hash-verified, append-only bounded
graph snapshots and cognitive events (schema migration 6). This is a local backend, not
the identity of the future distributed Archie database. Graph summaries are explicit
local state; they are not encrypted raw-vault contents.

`publish_cortex_graph` checks the expected parent and increasing publication time.
`apply_cortex_correction` validates the actual source/successor, preserves record IDs,
requires version advancement and exact correction-target agreement, and commits the graph
plus correction/trajectory in one transaction. Failed trajectory writes roll back the
graph. Identical retries are idempotent; original snapshots remain available by time.
The receipt-import API remains available for trusted external graph writers.

`CognitiveEvent` contains measured utility observations or historical Focus revisions.
Reads verify payload hashes and indexed identity/time. Bounded replay uses append sequence,
a captured high watermark and an as-of cutoff: later writes cannot change the replay set.
Unknown fields, nonfinite measurements, stale revisions and unsupported versions fail.
Silence is not successful task outcome; missing measurements remain absent.

Focus preserves objective references, parent lineage, suspension, pinned activations and
half-life decay. Parent changes are refused to prevent cycles and accidental reparenting.
Focus never writes graph facts. Rust encodes historical Focus in Cortex requests: requests
without Focus retain byte-identical v1 encoding; v2 adds a bounded attention extension.
Graph, response and refusal frames remain v1. Rust and C++ check shared wire fixtures.

`archie-core/src/cortex.rs` projects module declarations into namespaced process nodes and
dependency edges at boot. Unchanged manifests reuse representations; other knowledge and
Focus survive. Declarations are not running processes or execution grants. Linux exports
only public/local labels to `current.acg` with create-new, fsync, atomic rename and directory
fsync. Export failure degrades without blocking boot; the database stays authoritative.

`intelligence.hpp` separates cognitive nodes, versioned descriptors and shared analytical
heads. Descriptors declare inputs/outputs, dependencies, telemetry, tags, provenance and
authority references. Head attachment follows metadata, not known process names. The
bounded registry is a disposable view with no scheduler or policy capability.

Process predictions are empirical sample means and observed failure rates with per-output
minimum samples and temporal cutoffs. They are comparators, not trained neural models.
Duplicate evidence IDs and invalid measurements fail. Utility retains quality, relevance,
confidence, latency, tokens, CPU, memory, energy, privacy, risk and task success separately.
No unmeasured scalar objective is installed in the scheduler.

Consolidation proposes exact duplicate groups within kind/privacy class, excludes
contradicted nodes and retains every source reference. It never deletes evidence or merges
claims. `cognitive_projection` renders selected context as escaped Markdown. Automated
Obsidian synchronization and multi-note graph projection are not implemented yet.

### Reproducible validation

```sh
cmake -S cortex -B build/cortex -G Ninja -DCMAKE_BUILD_TYPE=Release
cmake --build build/cortex
ctest --test-dir build/cortex --output-on-failure
./build/cortex/archie-cortex-benchmark
cargo test --manifest-path core/Cargo.toml --locked -p archie-memory -p archie-state -p archie-core -j 1
```

The 512-node synthetic chain measures traversal correctness, not user answer quality.
At base `e06b4e3`, a three-hop query considered 512 candidates and selected 24 nodes
(2,590 rendered bytes). Baseline v2 considers/selects exactly four (489 bytes), retaining
the seed and all three allowed neighbors. One Windows/GCC 14.2 release run measured
p50/p95/p99 0.547/0.668/0.817 ms over 100 timed queries. These are local CPU measurements,
not RTX 3070, Qwen co-residency or production throughput claims. AF_UNIX E2E is explicitly
skipped on Windows and must run on Linux.

### Research decisions

- [DRIFT](https://microsoft.github.io/graphrag/query/drift_search/) and
  [dynamic community selection](https://www.microsoft.com/en-us/research/blog/graphrag-improving-global-search-via-dynamic-community-selection/)
  motivate bounded multi-cue exploration; no community index is fabricated from empty data.
- [Prospection-Guided Retrieval](https://arxiv.org/abs/2605.14177) uses prospective needs
  as retrieval cues. Adopt that separation from storage and evaluate against fixed-hop retrieval.
- [Human-Inspired Memory Architecture](https://arxiv.org/abs/2605.08538) and
  [PlugMem](https://arxiv.org/abs/2603.03296) motivate evidence-preserving consolidation.
  Broader knowledge compression needs replay quality gates.
- [TGN](https://arxiv.org/abs/2006.10637), [HGT](https://arxiv.org/abs/2003.01332) and
  [R-GCN](https://arxiv.org/abs/1703.06103) inform shared temporal/relational encoders.
  No real authorized training corpus or checkpoint exists here: maturity is DATA_STARVED.
- [RouteLLM](https://arxiv.org/abs/2406.18665), [FrugalGPT](https://arxiv.org/abs/2305.05176),
  [MixLLM](https://arxiv.org/abs/2502.18482), [MetaRouter](https://arxiv.org/abs/2606.06178),
  and [PILOT](https://aclanthology.org/2025.findings-emnlp.1301/) motivate separate quality,
  cost and affinity observations with dynamic candidates, without provider-specific wiring.
- [Mixture-of-Agents](https://arxiv.org/abs/2406.04692) and
  [AutoTool](https://arxiv.org/abs/2511.14650) inform composition/outcome contracts, not a
  fixed provider hierarchy or permission for unverified autonomous execution.

### Execution state and backend contracts

`archie-memory::storage` defines EventStore, ReplayStore, GraphStore, TrajectoryStore,
CheckpointStore and the composite ExperienceStore. The current `StateStore` is the
local adapter. Core module projection consumes GraphStore rather than a SQLite API.
These interfaces do not declare a distributed primary or invent a replication protocol.

Schema 7 adds immutable execution checkpoints and a local outbox. Each checkpoint
belongs to one Focus and branch, carries an explicit kind/status and complete retained
task references, and advances the Focus head by compare-and-swap. Rewinding requires
an explicit rollback on a fresh branch rooted in the active ancestry (or a new root).
Failed and abandoned siblings remain queryable history, never automatic working state.
Reads validate hashes, indexed identities, ancestor order and a 1024-frame path bound.
Checkpoint references cover decisions, verified-fact references, questions, artifacts,
tool results, plans and remaining work. They contain no hidden model chain-of-thought.
References must still pass evidence verification and privacy policy when resolved.

Core `cognitive_context::resume_context` validates the latest execution lineage, then
combines the current checkpoint with fresh constraints from a trusted ConstraintSource.
Authority, privacy, capabilities, operator constraints, invariants and approval references
stay outside retained task compaction. A historical timestamp cannot resurrect a prior
execution branch. This is a tested context-preparation API; a production policy adapter
and dispatch integration are still required. It does not grant execution authority.

Event, graph and execution publication enqueue immutable payload references and SHA-256
digests in the same transaction. Upgrade from schema 6 backfills existing references.
Bounded pending reads and exact-digest acknowledgements support a future authorized sync
adapter; no uploader runs, and acknowledgement never deletes evidence. That adapter must
resolve and validate the source payload and apply privacy policy before transmission.

[MAGE execution-state memory](https://arxiv.org/abs/2606.06090) motivates explicit ancestry
and branch revision. [CorpGen](https://arxiv.org/abs/2602.14229) motivates isolated per-task
contexts. [MemGym](https://arxiv.org/abs/2605.20833) motivates evaluating memory separately
from generative ability. Local regressions test rollback contamination, stale writers,
cross-Focus isolation, restart, corruption, migration, atomic outbox failure and fresh
constraints; no published benchmark score is claimed for ArchieOS.

### Bounded prospective retrieval

ContextRequest v3 adds up to eight explicit information-need cues grounded in node
IDs. The existing native compiler expands them in two synchronous rounds before its
bounded graph traversal. Each round sees the prior frontier, so input ordering cannot
extend the round limit. Every discovery retains its grounding path during packing;
inaccessible grounding cannot expose a visible downstream memory, and an over-budget
path is omitted as a group. Prospective cues never become graph facts or relationships.
The response identifies `prospective-v1` and explains this distinction. Requests
without cues retain `deterministic-v2`, with byte-identical v1/v2 request encodings.
See `protocols/cortex-context-wire.md` for v3 framing and shared Rust/C++ fixtures.

The synthetic distant-needs fixture has 512 nodes, no edges, and three required
memories. Original-query retrieval finds 1/3 using 54 estimated tokens. Explicit
two-stage prospective cues find 3/3 using 97 tokens, with zero irrelevant selections
in both cases. One local CPU run took 0.36 ms versus 1.23 ms. This is a deterministic
mechanism test with authored cues, not measured personalization or learned prospection.
Generating useful prospective needs from real task experience remains DATA_STARVED.
Tests cover two-round limits, order invariance, grounding closure, private bridges,
wire compatibility, malformed frames, context budgets and synthetic distant recall.

### Cerebra: structured shared thought and floor fallback

`SharedThoughtState` v1 retains objective, topic, Focus and execution-branch references
alongside bounded typed entities, facts, assumptions, hypotheses, constraints,
proposed actions, decisions, corrections, disagreements, open questions, artifacts
and tool results. Each item carries explicit evidence references and confidence.
Raw transcripts and hidden reasoning are not fields; unknown fields are rejected.
The record refers to evidence whose content remains behind its own privacy boundary.
An accepted assertion is collaboration state, not a privileged policy decision.

Local repair appends an evidence-backed replacement and marks the old item
superseded. It preserves objective/context and the old snapshot. SQLite schema 8
adds append-only, hash-verified thought snapshots, indexed by identity/revision/time.
Compare-and-swap and an atomic outbox receipt prevent conflicting writers or partial
synchronization intent. The migration preserves schema-7 data. Historical reads
remain available independently of the current head. `SharedThoughtStore` is a
separate backend extension; previous store adapters remain source-compatible.

Core's read-only `collaboration_context` combines the latest thought and active
Focus with freshly supplied policy constraints. Suspended Focus, future snapshots,
objective drift and stale constraint views fail. No transcript, action reference or
stored confidence grants execution authority. This is a context preparation API;
the speech/interactive producer and daemon control surface remain to be connected.

`ConversationalFloorState` carries probabilities, overlap, intent and utilities.
The current `decide_floor` is explicitly a conservative deterministic fallback,
not a trained model: acknowledgements with no question/request stay silent, ongoing
explanations retain the user's floor, stale/invalid observations fail silent, and
the trusted speech gate remains separate from utility. A trusted critical alert
can request interruption; estimator urgency alone cannot override a listen
preference. `can_speak` and `should_speak` are returned separately.

Replay fixtures cover “Okay”, explaining/overlap, a completed question, stale and
invalid observations, and critical alerts. Real streaming audio, learned floor
estimates, backchannel generation and Tribunal remain unimplemented. Learned
conversational competence is DATA_STARVED, never ACTIVE on the strength of these
authored tests. SharedThought durability is tested independently of that competence.
