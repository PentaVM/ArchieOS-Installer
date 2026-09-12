# Cerebra — the cognitive loop

Status: perception, learning admission and analytical synthesis are implemented kernel
contracts, and `archie-cerebra` now sequences them. No neural head exists; §7 is the
roadmap for adding one without giving it authority.

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

Nothing here is implemented. Each stage is listed with the types that already exist to
receive it, because the gates were built before the model deliberately — the deterministic
frame goes around the neural head, not after it.

### Stage 0 — today

No neural head. `gate_model_proposal` exists and would correctly accept or reject a
proposal; nothing produces one. The deterministic assessment stands alone, which is a
working system rather than a placeholder.

### Stage 1 — advisory interpreter

The first place a model enters Cerebra. It reads admitted evidence and proposes a stance;
`ModelProposal` and `gate_model_proposal` already define the contract it must satisfy.

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

### Stage 3 — floor estimation

`ConversationalFloorState` is documented today as taking estimates it does not learn:
`speaker_hold_probability`, `thought_completion_probability`,
`expected_silence_horizon_ms`, `interruption_utility`. These are the natural target for a
small local model, and they must stay local — turn-taking runs on raw audio timing, which
is exactly the material RFC-011 refuses to let leave the machine.

The decision function `decide_floor` stays deterministic regardless. A learned estimator
supplies inputs to it; it does not replace it, because a learned component that can
decide to interrupt has no bound on how often it does.

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

Until such a policy exists and is granted, Stages 1 and 2 are the reachable work, and
they are also the highest-leverage: better context selection improves every downstream
answer *and* reduces disclosure, because context not retrieved is context not disclosed.
Nothing about starting there forecloses the harder option later.

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
| Neural head (any stage) | Not started — §7 |
| Opportunity / intervention engine | Not started |
| Verification against world state | Contracts only |

`archie-cerebra` implements §2 and the orderings in §2.1, and its tests assert them
rather than describe them — a silent pass is checked against the vault's own audit log,
which must show that nothing was ever read.

What it deliberately does not do is call a model or execute a tool. `consider()` ends by
handing back a capsule and the identifier of the resource that may receive it; inference
and execution are I/O and live elsewhere. That is what keeps the whole loop testable
without a network.

The gaps that remain are §7 (no neural head), the opportunity/intervention engine, and
verification against world state, which has contracts but no implementation.
