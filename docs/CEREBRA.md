# Cerebra — the cognitive loop

Status: perception, learning admission and analytical synthesis are implemented kernel
contracts, and `archie-cerebra` sequences them. Advisory inference, context utility
learning and a local floor baseline are implemented; §7 records their validation limits.

Companion documents: [RFC-011](RFC-011-LEARNING-ADMISSION.md) (perception and learning
admission), [RFC-012](RFC-012-ANALYTICAL-SYNTHESIS.md) (bounded analytical synthesis),
[RFC-009](RFC-009-ANALYTICAL-CORTEX.md) (graph, Focus, experience).

---

## 1. What Cerebra is

Cerebra is the layer that decides **whether anything should happen**, and on what terms.

It is not a model and it does not contain one. Everything can call an LLM; that is not a
differentiator and it is not an architecture. What distinguishes Archie is the sequence
of decisions *around* the call:

- whether to speak at all, or stay out of the way;
- what may be disclosed in order to answer;
- which resource is permitted to see that;
- whether the resulting action is authorised;
- whether any of it deserves to be remembered.

Each of those is a decision over state, not a generation task. They are deterministic,
they are testable without a network, and they are the reason a model cannot quietly
become the operating system's authority.

The invariant, stated once:

> **Models advise. Cerebra decides. Policy authorises. Nothing else does.**

---

## 2. The loop

```text
observe ─→ floor ─→ contextualise ─→ route ─→ authorise ─→ act ─→ verify ─→ remember
             │                                                                  │
             └── most passes end here                     memory usually refuses ┘
```

The stage vocabulary is implemented in `archie_observability::cognition::Stage`, which
every subsystem projects its own receipt into. That projection *is* the internal
protocol: stages agree on this vocabulary rather than on each other's types. See
`docs/` and the `CognitiveTrace` documentation for the full list of eleven stages.

### 2.1 Two orderings that are load-bearing

**The floor is consulted before context is assembled.** Deciding whether to speak is
cheap and touches nothing private. Deciding *what to say* means decrypting vault entries
and building a capsule. In the other order, every time someone paused mid-sentence
Archie would read their private notes in order to conclude that it should stay quiet. A
pass that ends in silence must leave no capsule behind, because none was ever made.

**Privacy is a disqualifier, never a score.** `archie-router` runs two stages that are
never merged: hard disqualification, then scoring among the survivors. If privacy were a
heavily-weighted scoring term, a sufficiently attractive cloud model could outweigh it.
As a disqualifier it cannot.

### 2.2 Silence is an outcome

`StageOutcome` separates `Silent` from `Refused`, `AwaitingApproval` and `Failed`. These
are four different things a person would respond to differently, and a single "did not
happen" flag flattens them into one.

Silence is the expected ending for most passes of an always-on loop. A system that
cannot record a chosen silence cannot be told apart from one that broke quietly — which
is the difference between leaving it alone and restarting it. The failure mode of a
personal intelligence is not that it does too little.

---

## 3. Perception

Implemented: `sentry/` capture with bounded allocations, `protocols/capture-event.schema.json`,
`sentry/identity/wanda.{hpp,cpp}`.

Sentry and Wanda may observe continuously. What crosses out of perception may not be
what they observed.

- **Raw sensory payloads never leave this boundary.** A perceptual model may inspect a
  bounded buffer during inference; that does not make the buffer a memory. Audio, video,
  images and transcripts are discarded at the admission edge (RFC-011).
- **Volume is not knowledge.** Thirty frames a second and hours of background noise are
  not thirty corroborations. Repetition is explicitly not corroboration.
- **Wanda produces hypotheses, never facts.** Social inference is deterministic — whole
  permille rather than floating point, so identical evidence yields an identical integer
  on every node and a threshold cannot be crossed on one machine and not another.
  Confidence caps at 0.95 in the type's own constructor, because a mode that could reach
  1.0 would be making a claim about the world rather than summarising evidence. The only
  route from hypothesis to relationship runs through a participant declaring it, which
  rewrites the basis to `SubjectDeclared`; Wanda's evidence survives as a note in the
  margin. Repeated sightings saturate after four, so volume alone cannot reach a
  threshold given a long enough corridor.

The output of perception is a structured observation with provenance and consent
attached, and a short-lived working set. Nothing else.

---

## 4. Learning

Implemented: `archie-memory::admission` (RFC-011).

The admission firewall is what stands between an always-on perceptual system and a store
that grows without bound. It is deterministic and it is the default-deny direction.

A trainer-facing record contains **opaque references, SHA-256 digests, normalised
semantic keys and a supervision receipt**. It has no field capable of carrying audio,
video, images, transcript text or an arbitrary payload — the type cannot express the
thing the rule forbids, which is a stronger guarantee than a rule that asks.

Admission requires, jointly: verified supervision, consent where consent is required,
provider analysis permission, novelty above threshold, source quota headroom, and
durable exclusion from replay. Its refusal reasons are enumerated
(`RawSensoryPayload`, `ConsentAbsent`, `WandaIsUnconfirmed`, `SemanticDuplicate`,
`LowNovelty`, …) and each one is a distinct thing that went wrong.

**A refusal is not an error.** Most of what perception produces should be declined. A
loop that treated admission refusals as failures would, under pressure to make the
failures go away, end up removing the firewall.

---

## 5. Synthesis

Implemented: `archie-memory::analysis` (RFC-012).

Analysis separates a claim from the evidence for that claim, and stays useful only if it
refuses to treat reposted articles, repeated frames, or a model's own confidence as new
evidence.

- **One vote per independent provenance root.** Twelve outlets carrying one wire story
  produce twelve records and one fact. Getting this wrong does not merely inflate a
  number: it lets a single report acquire the authority of a consensus, on a screen that
  looks equally authoritative either way.
- **Source self-contradiction is quarantined** rather than averaged away.
- **Disputes are preserved.** The assessment states `unavailable`, `insufficient`,
  `supported`, `refuted` or `disputed` — a disputed question stays disputed rather than
  resolving to whichever side had more documents.
- **Independent corroboration is required** before a claim is treated as supported.

The deterministic assessment is computed first, and it is the answer.

---

## 6. Where a model is allowed to sit

Two places, both narrow, both already specified.

**Inside the ambiguous band, and only there.** Outside it Wanda has decided and no model
is consulted, because a settled question sent to a generative model comes back with a
confident opinion it had no need of. Inside the band a model may move the number *within*
the band and may not push it past either edge. Whatever the model reports about its own
certainty, the result is recorded as `Inferred`.

**As an advisory interpreter over admitted evidence.** `gate_model_proposal` accepts a
proposal only if it cites admitted evidence, clears the uncertainty thresholds, and does
not contradict the deterministic assessment. Its rejection reasons are explicit:
`InsufficientIndependentEvidence`, `UnsupportedEvidenceReference`, `InsufficientCitations`,
`ModelUncertain`, `ContradictsAssessment`, `NoStance`.

A model cannot establish evidence, erase evidence, grant a capability, or actuate
hardware. Those decisions live in the policy and execution boundaries and are reached
without consulting one.

---

## 7. Roadmap for the neural head

The stages below distinguish executable adapters from validated learned models. The
deterministic frame remains the authority around each estimator.

### Stage 0 — deterministic baseline

The deterministic assessment stands alone unless the optional runtime continuation is
called. It remains usable with no model, training data or network.

### Stage 1 — advisory interpreter

The first place a model enters Cerebra. It reads admitted evidence and proposes a stance;
`ModelProposal` and `gate_model_proposal` already define the contract it must satisfy.

Implemented: `archie-cerebra::advisory::AdvisoryInterpreter::interpret` continues a
`Pass` returned by `consider`, after floor/context/routing and before tool proposals or
remembering. It renders only a gateway capsule whose references survived analysis,
uses the router-selected descriptor through `archie-model-runtime`, and enforces 32 KiB
HTTP / 16 KiB proposal limits. Strict JSON and explicit content-free trace codes cover
malformed, oversized, uncited, unsupported and uncertain proposals. No model is called
for supported, refuted or unavailable assessments. A model-reported checkpoint digest
is syntactically validated, not a verified checkpoint attestation.

**Contract conflict:** the existing RFC-012 gate always abstains for disputed or
insufficient evidence and only accepts on settled assessments. With ambiguous-only
consultation, no runtime proposal can currently be accepted. This adapter preserves
that rule; it does not claim improved conclusions or corpus accuracy. A change to that
admission contract needs an explicit design decision. Loopback NIM-compatible tests
exercise the real transport; live NIM/Nebius accounts and a labelled corpus are not
validated here.

Success criterion: on a corpus where the deterministic assessment is `disputed` or
`insufficient`, the interpreter raises the proportion of questions a person can act on,
**without** a single accepted proposal that contradicts an assessment. The second half
matters more than the first.

### Stage 2 — context utility learning

`archie_memory::cognitive::TrainingEvent` and `UtilityObservation` already carry the
reward shape: `quality`, `relevance`, `confidence`, `latency_ms`, `tokens`, `cpu_ms`,
`memory_bytes`, `energy_joules`, `privacy_exposure`, `risk`, `task_success`, alongside
`LearningSignal::{ContextUseful, ContextIrrelevant, MissingContext, TaskOutcome,
ProcessSample}`.

That is a model of *which context was worth retrieving*, not of what to say. It is the
highest-value learning target in the system: better context selection improves every
downstream answer and reduces disclosure at the same time, because context not retrieved
is context not disclosed.

Note that `privacy_exposure` is a cost term. A policy trained to maximise task success
while ignoring it would learn to retrieve everything.

Implemented: `archie-memory::utility` learns integer empirical utility from
`AdmittedUtilitySample`. Binding requires a `CuratedTrainingRecord` produced by
`admit_learning`, matching event identity/reference/time and SHA-256 of the canonical
`TrainingEvent`. The trusted supervision producer must verify that digest before
admission; an arbitrary reward attached after admission fails binding. Admission's
eight-field record and firewall remain unchanged. Process samples have no attributed
context reward and are refused by this learner.

Reward is signal/task outcome plus half quality and relevance, minus twice privacy
exposure and risk (all quantised to whole permille). Missing cost measurements take
the worst case. Four updates globally and two for a reference are required to use its
score. Zero/insufficient training and expired parameters preserve the original request.
`Cerebra::with_context_utility` opts in after floor and before gateway retrieval; only
indices of requested references can survive. Missing-context signals cannot add context.

Parameters contain salted reference hashes, integer totals/counts and replay digests,
bounded to 128 references / 1,024 updates, with deterministic ordering and seed.
`archie-cerebra::parameters::{save_utility,load_utility}` writes and restores encrypted
local files under `Restricted`, with a fixed seven-day lifetime that training does not
extend. Expired state is refused; physical cleanup of encrypted files belongs to the
operator's checkpoint lifecycle (no background deletion job is installed).

Synthetic comparison only: both supplied contexts have successful outcomes; a
success-only selector keeps two references with 1,100 summed exposure permille, while
this learner keeps one with 100. These are fixture arithmetic, not measured accuracy or
a claim about a household. Real verified outcome ingestion, holdout evaluation and
deployment promotion remain operator integration work.

### Stage 3 — floor estimation

`ConversationalFloorState` is documented today as taking estimates it does not learn:
`speaker_hold_probability`, `thought_completion_probability`,
`expected_silence_horizon_ms`, `interruption_utility`. These are the natural target for a
small local model, and they must stay local — turn-taking runs on raw audio timing, which
is exactly the material RFC-011 refuses to let leave the machine.

The decision function `decide_floor` stays deterministic regardless. A learned estimator
supplies inputs to it; it does not replace it, because a learned component that can
decide to interrupt has no bound on how often it does.

Implemented baseline: `archie-speech::estimator` consumes fixed 320-sample mono
16 kHz windows (20 ms), extracts quantised amplitude, zero crossings and supplied local
silence timing, and zeroes the source buffer. Features and online labels have neither
`Serialize`, `Debug` nor `Clone`; features are zeroised on drop and never enter traces.
No audio, transcripts or labelled examples are persisted. Existing speech capabilities
and Sentry capture did not provide a PCM capture driver or labelled floor dataset;
this API accepts a device callback buffer but does not claim that driver is integrated.

Five bounded integer linear heads estimate hold, completion, resume, silence horizon
and interruption utility. Local task/session inputs supply response utility, urgency,
context sufficiency and explicit demand; sound alone cannot establish those semantics.
Invalid audio, invalid task estimates and expired parameters give conservative valid
estimates. The original deterministic `archie-memory::floor::decide_floor` is unchanged
and also fails silent on an injected invalid estimate.

`FloorEstimator::learn` consumes ephemeral features and human/verified targets,
updates only a fixed matrix, and supports encrypted seven-day `Restricted` checkpoints
via `archie-cerebra::parameters::{save_floor,load_floor}`. Constructors and restore
reject Cloud and Fabric. The speech dependency graph contains no network transport,
router or model runtime. Tests exercise online weight changes, deterministic replay,
extreme/NaN inputs, non-serializability, checkpoint restore and the unchanged floor loop.

This is a baseline plus an executable online learning interface, not a validated speech
model. No real labelled dataset, turn-taking accuracy, perception training, automatic
weight promotion or microphone integration is claimed. Nelson must choose a real
labelling/evaluation source; retaining a corpus still requires his separate consent and
retention decision. Online updates here retain no examples and need no such corpus.

### Training from Sentry: what the firewall already decided

Sentry is the richest signal in the system and the obvious thing to train on. It is also
the thing RFC-011 spends its entire length restricting. Both are true, and the resolution
is not a compromise — it determines which of the stages above are reachable today.

**What reaches a trainer from Sentry right now:** opaque references, SHA-256 digests,
normalised semantic keys, and a supervision receipt. The trainer-facing type has no field
capable of holding audio, video, images or transcript text. That is not a policy setting;
it is the shape of `CuratedTrainingRecord`.

So the question is not "can we train on Sentry" but "which models can be trained on
references and outcomes rather than on signal":

| Target | Trainable on admitted records? | Why |
|---|---|---|
| Context utility / salience (Stage 2) | **Yes** | Operates on which references helped and what the outcome was — exactly what `UtilityObservation` carries |
| Analytical interpretation (Stage 1) | **Yes** | Reads admitted evidence; `gate_model_proposal` already defines its contract |
| Relevance and novelty scoring | **Yes** | Semantic keys and digests are sufficient |
| Floor estimation (Stage 3) | **On-device only** | Needs raw timing, which may be inspected during inference but never retained |
| Perception itself — wake word, speaker ID, vision | **No** | Requires the signal, which admission destroys |

The last row is the real decision, and it is an operator's rather than an engineer's. A
perception model trained on this household's own data would be better than a generic one.
Getting there requires an explicit retention policy that keeps a bounded corpus, and that
policy needs, at minimum: per-subject consent rather than owner-only consent, a stated
expiry, and passage through admission anyway so that the corpus is curated rather than
merely accumulated.

Without that policy, stages 1 and 2 can operate on admitted references/outcomes, and
stage 3 can update weights from ephemeral local features and labels. Retained-signal
perception training remains unavailable. Better context selection can reduce disclosure
because context not retrieved is context not disclosed; measured answer improvements
still require real evaluation data.

What must not happen is the firewall being relaxed incrementally to make training easier.
It is load-bearing, it is the thing that keeps an always-on perceptual system from
becoming a surveillance archive, and it is far easier to widen once than to narrow again.

### Non-goals

- **No end-to-end neural policy.** A model that maps observation directly to action
  removes every boundary this document describes.
- **No model with authority.** Not over capabilities, not over hardware, not over what is
  remembered.
- **No learned privacy gate.** Privacy decisions are refusals, and a refusal that is
  sometimes wrong in the permissive direction is not a refusal.

---

## 8. The privacy spine through the loop

| Stage | Gate | Implementation |
|---|---|---|
| contextualise | Minimal retrieval, task-bound expiring capsule, pseudonymisation on external disclosure | `archie-vault::gateway` |
| contextualise | Secrets never enter context; they resolve at the execution boundary | `archie-vault::store` broker |
| route | Clearance, locality and verified retention, as disqualifiers | `archie-router`, `archie_types::{PrivacyGrade, DataRetention}` |
| act | Operator allowlist, capability grant, argument grade | `archie-mcp::gate` |
| act | Egress re-checks the routing decision | `archie-model-runtime`, `archie-mcp::transport` |
| remember | Admission firewall | `archie-memory::admission` |
| throughout | Trace records shape, never content | `archie_observability::cognition` |

`PrivacyGrade::Secret` is the grade no locality permits. Every other grade answers *where*
inference may happen; this one answers *whether*, and the answer is no.

---

## 9. Implementation status

| Component | State |
|---|---|
| Perception bounds, Wanda, capture contracts | Implemented |
| Learning admission firewall | Implemented |
| Bounded analytical synthesis | Implemented |
| Conversational floor decision | Implemented |
| Vault, context gateway, pseudonymisation | Implemented |
| Privacy-aware routing incl. verified retention | Implemented |
| MCP capability gate and HTTP transport | Implemented |
| Cognitive trace and HUD projection | Implemented |
| The loop that sequences them | Implemented — `archie-cerebra` |
| Advisory interpreter | Runtime adapter implemented; ambiguous proposals abstain under the existing gate — §7 |
| Context utility learning | Integer learner, admission binding, gateway ranking and encrypted checkpoints implemented; synthetic evaluation only |
| Floor estimation | Local integer baseline, ephemeral features/online updates, encrypted checkpoints implemented; no real dataset or capture driver — §7 |
| Opportunity / intervention engine | Not started |
| Verification against world state | Contracts only |

`archie-cerebra` implements §2 and the orderings in §2.1, and its tests assert them
rather than describe them — a silent pass is checked against the vault's own audit log,
which must show that nothing was ever read.

`consider()` does not call a model or execute a tool. It ends by
handing back a capsule and the identifier of the resource that may receive it; inference
and execution are I/O and live elsewhere. That is what keeps the whole loop testable
without a network.

The gaps that remain include §7's real-data evaluation, device integration and advisory acceptance contract, the opportunity/intervention engine, and
verification against world state, which has contracts but no implementation.
