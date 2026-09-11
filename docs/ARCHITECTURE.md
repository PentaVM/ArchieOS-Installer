# ArchieOS architecture

Status: current through RFC-009 bounded persistent graph/Focus/experience and atomic
correction publication, RFC-008 Sentry S0 and the
in-progress RFC-007 execution layer

Release target: next milestone release after `0.1.0-alpha.2` (not published)

Architecture line: Rust control plane on Arch Linux

Production naming adopted for the next implementation phases: **Archie** maintains
identity and continuity; **Cerebra** is the analytical/memory/Focus layer currently
implemented by Cortex and the Rust memory/state crates. **Sentry** observes health
and environment, with **Wolverine** as its planned recovery mode. **Watcher** owns
research, **Vision** visual interpretation with an Ultron profile, **Nova** simulation,
**Forge** engineering, and **Tribunal** selective critique. **Wanda** is a probabilistic
social-inference mode within Cerebra, not an authority or a detector of factual intent.
These names do not imply that every worker is implemented or permanently resident.
See `IMPLEMENTATION_STATUS.md` for the verified implementation and missing adapters.

The active data plane is PostgreSQL 18 with reviewed TimescaleDB and pgvector
extensions. There is no SQLite product-state fallback.
The independent Next.js/Vercel engineering dashboard lives under tooling and does
not replace the native Qt interface or enter the ISO. Policy and executor authority
remain deterministic and separate from all model suggestions and learned estimates.

## 1. Purpose and non-negotiable invariants

ArchieOS coordinates local compute, models, modules, and trusted network peers while
keeping the human operator in control. It is not a monolithic AI model and it does not
let a model become the operating-system authority.

The active architecture preserves these invariants:

1. **Operator supremacy.** Human configuration and approval are authoritative.
2. **Local-first.** Cloud use is optional and can never override privacy gates.
3. **Least privilege.** The daemon runs without Linux capabilities; workers will run
   outside the trusted computing base.
4. **Explicit authority.** Mutations require policy evaluation and a bounded lease.
5. **Physical safety.** No physical operation is authorized without an independently
   verified safety boundary; the default is absent.
6. **Persistent identity.** A node identity survives reboot and is never silently
   regenerated after corruption.
7. **Truthful capability reporting.** Hardware and peer state come from probes and
   verified announcements, not model claims.
8. **Explainability.** Module exclusions, scheduling deferrals, routing decisions, and
   policy decisions retain machine-readable reasons.
9. **Bounded resources.** Protocol frames, scheduler slots, daemon tasks, file handles,
   probing cadence, and release contents are bounded.
10. **Contract evolution.** A breaking contract requires an RFC/ADR and migration path.

## 2. System layers

```text
 operator / automation
          |
          v
  archie CLI (stateless, read-only v0.1)
          |
          | newline-delimited JSON; Unix socket, group-controlled
          v
 +---------------- trusted Rust control plane ----------------+
 | archied                                                  |
 |  boot/config  policy  registry  scheduler  router        |
 |  state/PostgreSQL  audit/metrics  hardware probe  fabric |
 +-----------------------------------------------------------+
          |                         |
          | signed fabric envelopes| future worker protocol
          v                         v
 trusted/pinned nodes       sandboxed modules/providers
          |                         |
          +----------- resources --+

 Arch Linux + systemd + filesystem + network + devices
```

Only `core/` is currently part of the trusted computing base. AI, research, desktop,
device, and provider processes must remain untrusted workers unless a later RFC makes
a narrower exception.

The local observatory is deliberately outside that authority path:

```text
fixed HTTPS feed ──> archie-worldd ──typed loopback JSON──┐
                       no Archie state                    v
archied ──read-only Unix control socket──> archie-displayd ──loopback──> native Qt 3D
 trusted                                 projection only                EGLFS/KMS
```

`archie-worldd` can reach its compiled-in authoritative provider endpoint but cannot
read Archie state or keys. `archie-displayd` can read the control projection and the
normalized world feed but has no Internet egress. The native graphics client can reach
localhost only. The renderer therefore never shares a process or network boundary with
authority.

The Analytical Cortex is likewise outside authority. It is a separate C++23 process that
reads a policy-filtered immutable graph snapshot and produces bounded context; it does
not share `archied`'s state database, control socket, identity key, process authority or
network namespace:

```text
trusted graph writer --atomic snapshot--> /var/lib/archie/knowledge/graph
                                             |
archied (UID-verified AF_UNIX client) --> archie-cortexd --> ContextBundle
                                             |
                                  local model router / worker (future)
```

Sentry is the analytical world model, and it is outside authority for the same reason
everything above is: it decides what may be believed and who may be identified, and holds no
power to act. It is a C++23 library of rules plus one program the owner runs and one optional
daemon:

```text
capture agents (owner's own devices)          cameras (RTSP)
 Android NotificationListener                       |
 browser webNavigation                              v
        |  NDJSON, owner's node only          sentry-camd (optional)
        v                                     holds credentials, decodes,
 sentry/capture gate ──> person graph <──     produces ANONYMOUS tracks
 suppresses non-owner    store (atomic)             |
                              ^                     v
                     archie-sentry (CLI)      sentry/vision gate
                     the owner's tool         names a track only if
                                              consent permits
```

Three rules are structural rather than advisory:

- **Identity resolution is consent-gated.** A camera sighting is an anonymous track; naming
  it needs a resolvable subject kind, the operator-subject-sensor consent intersection, and
  evidence — in that order, so a face model's confidence never decides whether the rule
  applies.
- **Detecting a non-owner suppresses capture rather than profiling them.** A phone handed to
  someone else records nothing about them.
- **Reading is a trust boundary.** The graph store re-runs every refusal on load, so a
  hand-edited or restored file cannot smuggle in a record the rules refuse today.

`sentry-camd` is built only where GStreamer and ONNX Runtime exist; the ISO does not include
it. `archie-sentry` ships. The capture agents are per-platform apps in `capture-agents/`,
side-loaded onto the owner's own devices and bound to the tested ingest by
`protocols/capture-event.schema.json`.

## 3. Runtime composition

`archied` is intentionally one daemon in v0.1. Crate boundaries separate decisions
without paying the memory, startup, and failure cost of many idle microservices.

| Crate/binary | Responsibility | Runtime state |
|---|---|---|
| `archie-types` | IDs, timestamps, nodes, capabilities, tasks, models, modules | Value types only |
| `archie-observability` | structured logging, counters/gauges, append-only audit contract | Shared bounded registries; durable audit via state store |
| `archie-state` | PostgreSQL persistence and filesystem layout | Identity, cognition, metrics, tasks, peers, audit |
| `archie-policy` | principals, resources, actions, decisions, leases, emergency pause | In-process authority engine |
| `archie-registry` | resource graph, module topology, model catalog | In-memory views rebuilt from durable/operator inputs |
| `archie-fabric` | Ed25519 identity, signed envelopes, discovery, peer liveness, transport | UDP in production profile; loopback simulator in tests |
| `archie-scheduler` | capability/trust/health placement, queues, reservations, preemption | Planning and running-job ledger |
| `archie-execution` | I/O-free worker specifications, lifecycle and executor wire protocol | Typed contracts only; no process or socket access |
| `archie-executord` | isolated process mechanism and delegated per-task cgroup owner | Supervisor/cgroup implementation; live `archied` transport integration pending |
| `archie-router` | deterministic model eligibility and scoring | Routing decisions; no providers bundled |
| `archie-memory` | provenance, cognitive contracts, deterministic learning admission | Bounded selection; raw sensor payloads excluded by type |
| `archie-cortexd` | C++ deterministic graph-context compiler | Socket-activated, bounded Focus/multi-hop context; trusted module projection feeds the graph; neural heads DATA_STARVED |
| `archie-core` | boot orchestration, hardware probe, control API, integrated runtime | Owns subsystem lifecycle |
| `archied` | foreground daemon, control and fabric loops, signal shutdown | One process per state root |
| `archie` | thin CLI client and human/JSON rendering | No independent state |
| `archie-displayd` | read-only control/world projection; no UI or asset serving | Bounded last-known-good display cache; no Internet egress |
| `archie-worldd` | fetch and normalize fixed authoritative physical-world feeds | Bounded provider cache/backoff; no Archie state or control access |
| `archie-display` | compiled C++/Qt Quick 3D native observatory | EGLFS/KMS system client; no keys, control socket or public egress |

## 4. Boot and shutdown

The runtime boot is deterministic and records each stage in a `BootReport`:

```text
paths/directories
  -> configuration validation
  -> persistent Ed25519 identity
  -> PostgreSQL compatibility/schema/state
  -> real hardware probe and capability derivation
  -> durable boot record and capability delta
  -> resource graph with local node
  -> module topology compilation
  -> policy, scheduler, router, metrics and audit wiring
  -> fabric transport
  -> ARCHIE READY
  -> bind exclusive control endpoint
```

A missing configuration is safe and yields a `dev-vm` loopback node. A malformed file,
unknown key, invalid network address, unsafe timeout, or impossible scheduler
reservation is fatal. The live ISO deliberately opts into UDP while retaining the safe
`dev-vm` role.

At shutdown the daemon announces departure, stops background fabric work, drops the
control listener and its exclusive lock, removes ephemeral endpoint files, and exits.
Crash leftovers are recoverable, but a second live daemon cannot replace the endpoint.

## 5. Persistent and ephemeral state

Default Linux layout:

| Path | Contents | Lifetime |
|---|---|---|
| `/etc/archie/archie.toml` | operator/installer configuration | Persistent, human-owned |
| `/var/lib/archie/identity/node.key` | Ed25519 private node identity | Persistent, secret |
| `/var/lib/archie/archie.db` | boots, capability deltas, tasks, peers, trust, audit and hash-verified Cortex correction trajectories | Persistent |
| `/var/lib/archie/cortex/` | derived Cortex cache and future registry state | Persistent, unprivileged |
| `/var/lib/archie/knowledge/graph/` | trusted immutable temporal graph snapshots | Persistent, read-only to Cortex |
| `/var/lib/archie/private/` | raw private vault allocation | Persistent, inaccessible to Cortex baseline |
| `/run/archie/archied.sock` | local control endpoint | Ephemeral |
| `/run/archie/cortex.sock` | UID-verified Cortex AF_UNIX IPC | Ephemeral, socket-activated |
| `/run/archie/archied.lock` | OS-backed single-instance lock | Ephemeral authority |
| `/var/log/archie/` | service-owned log directory | Persistent allocation; active logs go to journald |

`--root DIR` relocates the complete layout for tests and multiple local simulated nodes.
On Windows development hosts, the control endpoint is loopback TCP with its address
published beneath the relocated runtime directory.

PostgreSQL owns schema migration, boot history, task serialization and state transitions,
TOFU peer observations/trust, append-only audit persistence, Timescale metrics and
versioned pgvector embeddings. It also stores bounded temporal graph snapshots, atomic
corrections and historical Focus/learning events.
The encrypted vault and retention lifecycle remain separate Wave H work.

Ambient perception is not written straight into cognitive history. The deterministic
admission boundary in [RFC-011](RFC-011-LEARNING-ADMISSION.md) keeps Sentry observations
in a bounded working set, keeps Wanda hypotheses temporary, and exposes trainer input only
for receipt-backed operator corrections or verified task outcomes. Raw audio, video,
images and transcripts have no representation in that trainer-facing type.

The analytical synthesis boundary in [RFC-012](RFC-012-ANALYTICAL-SYNTHESIS.md) then
reduces sanitized signals to one vote per independent provenance root, preserves
contradictions and makes unavailable, insufficient and disputed evidence explicit. Neural
heads may attach a cited advisory interpretation; they cannot establish a graph fact from
their own score or from repeated ambient input.

## 6. Local control plane

The CLI sends one bounded newline-delimited JSON request and receives one response.
Linux authorization currently comes from access to the `archie` group-owned Unix
socket (`0660`). Each client is served independently so a stalled client cannot block
another operator query. Frames larger than 1 MiB are rejected.

All v0.1 requests are read-only:

| Request | Result |
|---|---|
| `ping` | liveness, node ID, version |
| `status` | identity, host, hardware, resources, health and totals |
| `topology` | nodes and capability-to-node index |
| `modules` | installed state, load order and exclusion explanations |
| `peers` | trust, reachability, health and capabilities |
| `models` | model/provider/locality/privacy/availability catalog |
| `boot` | stage-by-stage report and warnings |
| `metrics` | point-in-time counters and gauges |
| `audit` | recent durable authority records |
| `cortex-trajectories` | receipt-bound correction/replay metadata only; no vault reference, parser, graph write or model promotion |

Mutating commands are intentionally absent. Their future protocol must authenticate
the principal, call policy, persist an audit decision, issue or validate a scoped
lease, and only then invoke a worker. Possession of the socket alone will not be enough
for broad application/device authority.

## 7. Fabric and node trust

Each node has an Ed25519 key. Its `NodeId` is derived from the public key. Discovery
messages contain a self-consistent node descriptor in a signed envelope capped at
64 KiB.

The receiver verifies:

- envelope version and size;
- signature;
- public key to `NodeId` binding;
- descriptor identity consistency;
- timestamp/liveness rules.

First valid contact is **pinned** through trust-on-first-use. Trust is always owned by
the receiver; a peer cannot declare itself trusted. Explicit promotion is required for
sensitive work. Silence changes effective health and eventually removes a peer from
the live graph. Durable observations remain available for operator review.

`Transport` is replaceable. The implementation includes UDP and a deterministic
in-process loopback bus supporting partition/heal simulation. Zenoh is not implemented.
The signed descriptor is authentication, not remote hardware attestation.

## 8. Resources, modules, scheduling and routing

### Resource graph

The graph combines the real local hardware probe with accepted peer descriptors. It
indexes CPU, memory, accelerator, storage, network, profile, trust, health and named
capabilities. Only healthy, sufficiently trusted nodes are schedulable.

### Module topology

Module manifests express implementation kind, dependencies, architecture/profile
support, required capabilities, permissions, and health checks. The topology compiler
detects invalid manifests and dependency cycles, orders loadable modules, honors
operator disablement, and explains every exclusion.

The execution chain is submit -> queue -> plan -> dispatch -> executor -> process,
and every link now exists as tested logic: a pending-task queue that derives readiness from
the dependency graph and cancels what a failure made impossible; a scheduling pass that plans
and dispatches; a bridge to the executor that never retries a start whose outcome is unknown,
because a retry there is how a task runs twice. `TaskExecutionService` composes them and is
tested end to end against a scripted executor.

What remains is exposure, not mechanism: a submit control request, a tick on the daemon's
timer, and a registry-backed `SpecSource` that turns a module's `[worker]` declaration into a
runnable executable. That last one is a worker-execution-model decision rather than glue, and
it is the real gate on the chain running end to end.

### Scheduler

Tasks are a validated dependency graph with explicit state transitions and retry
limits. Queues are `active`, `background`, and `deep-idle`. The scheduler reserves
capacity for active work, selects nodes by capability/trust/health/load, and plans
checkpoint-before-preemption where required.

### Execution

The scheduler decides; it does not spawn. Execution is a separate privilege domain,
split across three processes so that `archied` never needs the access a worker
supervisor requires (RFC-007 §7):

| Component | Role | Privilege |
|---|---|---|
| `archied` | authority — decides *what* runs | unchanged; empty capability bounding set |
| `archie-executord` | mechanism — turns a decision into a process | one delegated cgroup subtree, nothing above it |
| `archie-task-runner` | sandbox — applies the irreversible boundary, then `exec`s | drops everything it does not need |
| worker | the workload | untrusted |

The runner is separate from the executor because Landlock is irreversible for a
process tree: whatever applies it must be the process about to *become* the worker,
not a long-lived supervisor with other tasks still to start.

`archie-execution` holds the contracts and is entirely I/O-free. A
`TaskExecutionSpec` has nowhere to put a shell command — the executable comes from
the trusted module registry, and `ResolvedExecutable`'s fields are private with one
constructor. The worker's environment is an allowlist, so credentials the daemon
holds stay in the daemon. And a spec whose required enforcement this node cannot
provide is refused with the shortfall named, rather than run here in a weaker form.

### Model router

Privacy, availability, capability, context, output, quality, and cost are eligibility
gates. Eligible candidates receive deterministic quality/latency/cost/locality scores.
Restricted work cannot be routed to an impermissible cloud model, regardless of score.
`RemoteHarness` produces a server-resident plan only after that selection; it carries an
input digest rather than prompt text in routing/audit data and has no provider endpoint
or credential field. A paired device can be named only as a typed result target, never
as an inference placement. No real model provider is bundled or registered by default.

## 9. Policy, permissions and physical control

Policy describes a principal, requested scope, resource, action and constraints. A
decision may allow, narrow, deny, request approval, or hard-stop. Granted authority is
represented by a revocable lease with expiry, usage limit, scope, resource and action
bounds. Emergency pause rejects mutations.

Physical actions are distinguished from software/data actions. They require:

1. an authenticated human or explicitly delegated principal;
2. an operator policy decision;
3. a valid scoped lease;
4. a configured and verified external safety boundary;
5. a durable audit record before actuation.

For device mutations, a valid lease is deliberately narrower still: it must name one
prepared operation, require presence when used, allow exactly one dispatch, and expire
within five minutes. An authenticated operator can approve a concrete change but cannot
mint a broad or standing remote-control capability.

The current runtime performs no physical actuation and has no universal popup/app
permission broker. Those are future components and must not bypass operating-system
security prompts or silently approve high-impact requests.

The next device boundary is deliberately narrower than a remote-administration system:
only an explicitly paired device agent will receive a fixed typed operation after
`prepare -> policy/approval -> scoped lease -> dispatch -> receipt`. Policy now has an
explicit `Scope::OneOperation` check that the future device dispatcher must use against
the prepared operation identifier, so that integration cannot treat a one-change grant
as broad authority. The generic lease validator refuses that scope outright, making an
omitted prepared-operation identifier a visible failure rather than an accidental allow.
The server retains model inference and planning; a device agent validates bounded
requests and reports state. The native core now has a no-I/O television preparation and
dry-run contract: it accepts only HDMI ports 1–16, Roku, tuner and bounded channel
operations; ties each proposal to one operation id; and maps it to the existing
physical-policy request. Voice is only a digest-backed proposal, never confirmation.
Its deterministic simulator predicts a receipt without opening a network connection or
mutating a device. The local state store also has a durable, key-pinned device inventory:
an explicitly supplied proposal begins `proposed`, local confirmation transitions it to
`paired`, and revocation is sticky. Reappearing with a different key or device class is
refused. The television dry-run admits an inventory-backed target only when it is both
`paired` and classed as a television. This inventory stores no endpoint or credential
and enables no I/O. The native read-only `archie devices` projection exposes its
reference, class, pairing state, key fingerprint and observation times without exposing
the full key. The equally native read-only `archie device-operations` projection exposes
the operation id, state, time and short trace/digest fingerprints, never a payload,
lease, endpoint or command path. A durable operation journal allocates the exact
operation id, binds it to a trace and canonical payload digest, and permits only
`prepared` or permanently `cancelled`; it retains no payload or live authority after
restart. Native device-adapter
manifests are now a separate, fixed contract: they name only a built-in ISO module,
bounded device classes and a bounded operation vocabulary. A generated adapter is a
reviewable proposal only and cannot activate; it can be generated only from these typed
requirements and an explicit resource budget. The manifest cannot carry an endpoint,
credential, shell command, raw frame or firmware image; a mutating adapter must declare
dry-run and receipt reconciliation, and firmware apply additionally requires inspect,
stage and rollback capabilities. The local adapter catalogue stores generated proposals
and reviewed native contracts separately, preventing a generated id from being shadowed
by later code; generated proposals are also persisted locally as hash-verified history.
That history cannot load a worker or open I/O. A native contract can
be admitted only against a
currently paired device of its declared class; admission creates no transport, lease or
dispatch authority. Each future dispatch must then bind the declared operation to the
durable identifier/digest and validate a fresh exact one-operation lease; this common
contract maps mutations to physical `Actuate` and observation to typed `Read`. Router
firmware, raw packet capture, camera stream,
CEC/Roku/screen transport, signed pairing exchange, recovery execution and approval
persistence remain absent from the active ISO.

The next read-only network boundary is a native, structured Nmap survey plan rather
than a general shell facility. It accepts one to 32 explicitly declared, canonical
RFC1918 IPv4 CIDRs, rejects host-address spellings and duplicate networks, and caps one
survey at 65,536 addresses. Its only profiles are host discovery and a rate-capped,
light common-port service inventory; neither accepts scripts, credentials, exploits or
arbitrary flags. The current plan is I/O-free: a future sandboxed runner must first
bind it to a local-network engagement, record normalized evidence, and retain a
verifiable audit record. Evidence first flows through a deterministic, duplicate-safe
classifier for normalized router, mDNS, SSDP and port observations; it produces only a
candidate inventory class with a score and never pairing or control authority. A
generative system may explain that result later. A completed survey is accepted only
when every unique observation is inside that plan's declared private CIDRs; the result
is address-ordered and an unknown responding host stays unclassified. Every signal also
declares a fixed provenance: only service inventory can yield an open-port signal, while
mDNS, SSDP and router advertisements must be explicitly passive. Two valid observations
of the same declared scope can then be compared deterministically for newly observed
assets, changed classifications and assets no longer observed. The latter is never an
offline/trust claim. No Nmap runner,
packet capture, general network scan or control path ships in the active ISO yet.

## 10. Observability and resource posture

The daemon emits structured JSON logs to journald and the console. Metrics use atomic
counters/gauges and are exposed through the local control endpoint. Authority events
use an append-only audit API and PostgreSQL storage. Routing and topology decisions retain
explanations.

Resource controls currently include a single authority daemon, 60-second hardware refresh,
delayed missed ticks, bounded frames, fixed scheduler slots, systemd `TasksMax=256`,
`LimitNOFILE=8192`, release optimization/LTO/stripping, and an ISO without Python,
model weights, sources, or debug tools. The optional observatory has its own CPU/RAM/
task/file-descriptor envelope and event-driven rendering; its pressure cannot make it
an authority dependency. Continuous cross-platform worker RSS/CPU/GPU feedback and
adaptive throttling remain in the worker/resource-governor milestone.

## 10a. The observatory's visual model

Specified in [RFC-010](RFC-010-OBSERVATORY-VISUAL-MODEL.md).

The observatory is split in two, and the split is the design rather than an accident of
layout. `display/visual` decides *what* is shown; `display/native` draws it. The
deciding half links no Qt, opens no socket and reads no clock, which is what makes it
testable on a machine with no GPU — and a decision that can only be checked by looking
at a running window is a decision that will not be checked.

The rules it owns all come from the same commitment the rest of ArchieOS makes: the
display may not claim more than it knows.

- **Freshness is a visual channel.** Every panel is a claim about a machine somewhere
  else, and a stale value looks exactly like a fresh one until someone acts on it. Age
  costs confidence, and animation stops before data disappears, because motion is what a
  viewer reads as "happening now". A timestamp from the future is reported as a broken
  clock rather than rewarded.
- **The render budget comes from the scheduler's own evidence.** Kernel pressure and
  thermal telemetry decide the tier, so the display yields for the same reason and at the
  same moment the scheduler does. A rich tier requires positive evidence of headroom: an
  unreadable sensor, an available flag with no value behind it and a four-minute-old
  reading all resolve cautiously. A saturated machine is never blanked, because that is
  when an operator most needs the screen.
- **Node identity survives refreshes.** A node is its id, not its index; it holds a
  layout seat for its whole life; and one absent poll starts a grace period rather than a
  death, so a module that restarts between polls does not visibly die and get reborn.
  This is what makes any smooth transition possible at all.
- **Position means something.** Radius comes from dependency depth, so the scene reads
  outward from its foundations. Angle comes from the node's slot through the golden
  angle, so arrivals fill gaps instead of pushing everyone. Height comes from severity.
  Nothing is hashed.
- **Colour carries severity twice.** Hue and luminance both move with it, so the ordering
  survives greyscale and colour blindness. Age never touches hue: a stale critical node
  is still a critical node.
- **Events are derived, never invented.** Every pulse comes from a graph delta. The queue
  is bounded, coalesces per node, and counts what it dropped.
- **The operator owns the view.** Nothing switches modes on an alarm or an arriving
  observation.

`Observatory` composes these into a `Frame`: pure data the renderer applies, with no
decision left for it to make. It is the single entry point the Qt layer is meant to call.

Status: the model is implemented and tested — 14 suites, 6934 checks, green under
`-Werror` with no Qt. The QML that consumes it is not written, so none of this reaches a
screen yet.

## 11. Arch Linux integration and release

The `archie` system account owns state and runtime directories. `archied.service` runs
as `Type=simple`, starts after the network is online, restarts on unexpected failure,
does not loop on configuration exit code 78, and applies systemd hardening with an
empty capability set.

The ArchISO profile supports BIOS/syslinux and UEFI/systemd-boot. The build compiles
locked stripped x86-64 ELF binaries, stages only allowlisted release content, enables
the daemon/network services, invokes `mkarchiso`, and generates `SHA256SUMS`. GitHub
Actions builds the control plane and boots the exact ISO in QEMU using TCG before a
release workflow may publish it. `v0.1.0-alpha.1` and `v0.1.0-alpha.2` passed that
gate. The graphical session is fail-open and conditional on a DRM device, so headless
control-plane acceptance remains independent from monitor/GPU availability; it is not
physical DRM/seat evidence.

## 12. Compatibility boundaries

- Wire structs are version-sensitive contracts even where a separate version field is
  not yet present. Breaking changes require an RFC and compatibility test.
- Module manifests, task state transitions, identity derivation, trust ownership,
  filesystem paths, and CLI exit codes are public behavior.
- Historical Python/PySide6 implementations remain in the source archive and are not a
  dependency of `archied` or this repository.
- Reserved top-level directories do not prove a subsystem is implemented.
- A release may claim headless boot verification only for the exact artifact whose QEMU
  log reached `ARCHIE READY`; graphical hardware verification is a separate claim.
