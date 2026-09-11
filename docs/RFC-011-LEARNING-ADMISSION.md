# RFC-011 — Perception and learning admission

Status: implemented kernel contract; runtime adapters pending

Sentry and Wanda may observe continuously. Model training must not. A camera can emit
thirty frames each second and a microphone can hear hours of silence, repeated words and
background noise; volume is not knowledge and repetition is not corroboration.

The implemented boundary is:

```text
sensor bytes ── bounded inference buffer ──> Sentry structured observation
     │                       │                         │
     └─ destroyed/retained   └─ no general memory    ├─ working set (short-lived)
        under owner policy                             └─ verified/corrected derivative
                                                               │
Wanda evidence ──> expiring social hypothesis ── human confirms/corrects
                                                               │
                                                               v
             deterministic admission firewall ──> curated references ──> trainer
```

`archie-memory::admission` is the firewall. Its trainer-facing type contains opaque
references, SHA-256 digests, normalized semantic keys and a supervision receipt. It has no
field capable of carrying audio, video, images, transcript text or arbitrary payloads.

## Admission rules

1. Raw audio, video, images and transcripts are discarded at this boundary. Perceptual
   models may inspect bounded buffers during inference; that does not make those buffers
   memories or training samples.
2. Sentry observations are context, never supervision. They remain in the working set
   until deterministic fusion produces a useful derivative.
3. Wanda hypotheses remain in the working set and expire. They never train a model and
   never become asserted relationships without a participant declaration.
4. Training requires either an operator correction with its graph-writer receipt or a
   task outcome with a verification receipt. The caller supplies the set of receipts
   already verified against trusted state; merely naming a receipt in input is insufficient.
   Model-generated labels cannot supervise the same model.
5. Provider `machine_analysis_allowed`, active consent and sensitivity gates run before
   scoring. Biometric and secret data are always rejected; exact location is never used as
   training material. Personal data is disabled for training by default.
6. Whole-permille quality, relevance and novelty floors remove weak samples. Semantic
   deduplication, a durable set of previously admitted evidence digests, per-source quotas
   and a total batch quota prevent a loud source from dominating the current batch or
   replaying the same sample into later batches.
7. Every candidate receives a machine-readable disposition and reason. Rejected data does
   not become a negative training example merely because it was rejected.

## Neural architecture

The model stack has three separately versioned roles:

- Perception heads turn bounded local sensor windows into structured measurements. They
  run inference-only against ambient streams.
- Cerebra retrieves verified graph state and short-lived context. Model weights are never
  the source of truth.
- Training adapters consume only `CuratedTrainingRecord` batches, balance them by source
  and semantic key, evaluate on a frozen holdout, and enter shadow mode before promotion.

This architecture improves learning quality by making the dataset smaller and more
informative. It does not claim that a neural network can infer consent, truth or importance
from raw volume; those decisions stay deterministic and auditable.

## Bounds implemented at capture

Sentry capture batches are limited to 4 MiB, 16,384 parsed/unparsed events, 4,096 bytes per
line, 192 bytes per identifier and 512 distinct contact labels. Repetition is summarized
as saturating counts. If a bound is hit, `input_truncated` or `contact_capacity` records the
loss without scanning the discarded tail.

The next runtime step is to serialize Sentry aggregates and Wanda hypotheses as
`learning-candidate.schema.json`, then persist admission decisions, admitted evidence
digests and expiry jobs in PostgreSQL. No current trainer is therefore claimed to be
operational.
