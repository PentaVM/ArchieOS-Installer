# Wanda LNCM: a local next-concept expert for social context

Wanda answers questions about people: who knows what, what someone is likely to do, how to say
something to them. The LNCM is the small local model that does this after a language model
and before the voice. It reads conversations of any length in chunks, keeps a persistent
state per person, and answers in two states: a closed answer at once, then a refined long
answer a few hundred milliseconds later.

- **L**: low-rank adapters per context domain, trained without touching the shared weights.
- **NCM**: next-concept modelling ([Hazy Research, arXiv:2610.14162](https://arxiv.org/abs/2610.14162)):
  text is a hierarchy of discrete codes, coarse to fine, and each level is predicted in one
  parallel pass instead of one token at a time.

Code: `core/crates/archie-wanda-lncm` (model, state, reader, data, distillation, training, CLI)
and `core/crates/archie-wanda/src/social.rs` (service integration). Training guide:
[`training/wanda_lncm/README.md`](../training/wanda_lncm/README.md).

## Architecture

```text
═══════════════════════ OFFLINE · Nebius AI Cloud / Token Factory ═══════════════════════
 Cases: owned synthetic worlds (synth) · Python tomgen tranches · public licensed datasets
   │   admitted only if owned/public, no credential/e-mail/phone/card/RUT shape,
   │   no 8-word overlap with evaluation-only sets (FANToM, OpenToM, …)
   ▼
 Teachers (Nemotron, DeepSeek, Qwen, Kimi … via the registry and InferenceClient, PrivacyGrade::Public)
   ▼
 Selection: exact simulator label ▸ majority / recurring delta lines ▸ cross-family judges
   ▼
 Examples (draft → expert pairs) ──► init (BPE codec) ► tokenizer (multiscale VQ-VAE)
                                      ► generator + heads ► adapters per domain ► calibrate
══════════════════════════════ RUNTIME · archie-wanda, local ═════════════════════════════
 WhatsApp / notification events ─► Vault (Restricted, per domain)
   ▼ new events since the cursor
 Reader: chunks ─► exact bookkeeping (who was present for which message, reply latency)
               └─► LNCM update head + delta lines ─► validated against the chunk ─► state
   ▼ persistent SocialState (Vault, Restricted, per domain): seen values, expectations, mood
 Question ─► crisis rule? ──yes──► fixed message, no model
   │ no
   ▼
 Cerebra/router: local reasoning model (draft recommendations)
   ▼
 LNCM gate ─► state 1: does X know / what does X believe / an expectation it names?
          │            yes ─► read from the state, with the reason            ─► voice now
          │            no  ─► decision head (yes / no / cannot know + confidence)
          └─► state 2: NCM generator (plan, then words), name guard           ─► voice next
   ▼
 Inferences stay hypotheses; Cerebra ▸ Policy ▸ lease ▸ Stage hold every authority they had.
```

## Components

| File | What it is |
|---|---|
| `codec.rs` | Byte codec and a BPE trainer/encoder in Rust. Special tokens are never parsed from text. Prompt builder with water-filled budgets. |
| `vq.rs` | Multiscale residual VQ-VAE: per-level residual quantisation at `ℓ_k` positions, straight-through gradients, commitment and codebook losses, nested dropout. |
| `generator.rs` | Prefix-LM transformer: the prompt reads itself; each level's block reads the prompt and itself. All levels are trained in one pass from corrupted coarser codes (eq. 10 weighting); generation runs one pass per level with the prompt's keys and values cached. Heads: gate, yes/no kind, chunk change, decision with abstention. |
| `nn.rs` | Linear layers with optional LoRA (zero-initialised `B`), RMSNorm, attention with additive masks and prefix cache, GELU MLP. |
| `model.rs` | Checkpoint loading with SHA-256 verification of every file and of each adapter against its base; `gate`, `decide`, `quick`, `compose` (MBR over candidates, name guard), `update`. |
| `state.rs` | `SocialState`: exact exposure per person and topic, expectations as decayed Beta counts (capped at 0.95 unless declared by the person), mood, behaviour statistics; the delta-line grammar and its validation; "pack v3" rendering. |
| `exact.rs` | State 1 from the bookkeeping: "does X know …", "what does X believe about …" and questions that name one of X's expectations are answered from the state, with the reason; a person the conversations never show saw nothing in them. Anything it does not recognise with certainty goes to the decision head. |
| `reader.rs` | Chunked read–update loop with constant memory, idempotent by turn number, records chunks for training. |
| `synth.rs` | Owned ES/EN worlds with hidden traits, fact changes while people are absent, "I already told X", moods; exact labels for all four tasks. |
| `data.rs` | Closed JSONL schemas (`Example`, `Case`), provenance rules, Python `chat-v1` tranche import, evaluation-overlap guard. |
| `distill.rs` | Teachers, drafts, judges, selection; resumable, concurrent, budgeted. |
| `train.rs` | Stages, AdamW with warm-up and cosine decay, global-norm clipping, dead-code restarts, resumable saves, evaluation with measured latency, temperature calibration. |
| `safety.rs` | The crisis rule: a fixed message, no interpretation. |

## Invariants

1. Nothing private trains anything or reaches a teacher. Cases must be owned or public; the
   Vault's veil refuses any case with a sensitive shape; teacher calls go through
   `InferenceClient::chat` at `PrivacyGrade::Public`.
2. Benchmarks are evaluation-only. Cases overlapping them by eight words are refused.
3. A model proposes; validation decides. A delta line about a person not in the state, or a
   turn outside its chunk, is rejected. Names in generated answers are checked against the
   state; unverified names keep the answer from being spoken.
4. Inferences are hypotheses. Confidence about a person is capped at 0.95 unless that person
   declared it; the owner can confirm (still a hypothesis), retract, or forget a person.
5. Domains stay apart. Each domain has its own state entry and may have its own adapter.
6. Weights are served only as measured: every file is checked against the manifest at start;
   an adapter is refused if the generator changed after it was trained.
7. A crisis is never interpreted.

## Service integration

Optional `[lncm]` section in Wanda's config:

```toml
[lncm]
dir = "/var/lib/archie/wanda/lncm"   # checkpoint written by archie-wanda-lncm
device = "cpu"                        # or "cuda:0" when built with --features cuda
lang = "es"
gate_threshold = 0.5
update_threshold = 0.5
owner_label = "Yo"

[lncm.adapters]
"personal/owner" = "personal-owner"
```

- The proactive refresh reads new conversation events into each domain's state. Collectors
  do not report who is in a group, so membership is what the events show: whoever has written
  in a conversation is taken to be in it, and the owner always is. "Who saw this" is exact
  for the synthetic worlds and an approximation for real chats until a collector reports
  members as join/leave turns.
- `POST /v1/query` returns `quick` (state 1) and `refined` (state 2) when the question is
  social; `speech_text()` says state 1 only above 0.6 confidence and state 2 only when no name
  is unverified, and otherwise falls back to the existing recommendations.
- `GET /v1/people?domain=…` shows what to expect of each person; `POST /v1/people/edit`
  confirms or retracts an expectation or forgets a person (owner token).

## What is verified, and what is not

Verified here, on CPU, by tests in this repository: every code path above, including a full
tiny training run (codec → tokenizer → generator → adapter → calibration → evaluation →
serving → reading), checkpoint tampering being refused, the distillation pipeline against
loopback stand-ins for teachers and judges, and the service reading events into a persisted
Vault state.

Not verified: any trained model's accuracy, any benchmark result, latency on any GPU, or CUDA
builds. No checkpoint is shipped. Numbers come only from `archie-wanda-lncm eval` on real
training runs, recorded in the checkpoint's manifest with the data they came from.
