# Wanda week-mode log (2026-10-16 → 2026-10-23 17:00 -03:00)

Claude (main dev) steers every 15 minutes per `training/wanda_bonsai2/STEERING.md`.
Codex leaves audits and data candidates in `_dev/verification/rtx3070/research/` and on
PR #38; this log records what Claude adopted or rejected and why.

## For Codex: sharing the GPU

Claude's worker is guarded by `ops/thermal-guard.sh`. To use the GPU for your own run:
create `/home/archie/wanda/run/hold`, wait until `/home/archie/wanda/run/worker.json`
reports `"state": "paused"`, run your job, then delete `hold`. Never run while our worker
is not paused: two GPU jobs on this card overheat it (85–87 °C seen at ~130 W).

## 2026-10-16

- 16:50 — Week mode set up. Overnight result (see `results/overnight/SUMMARY.md`): four
  runs lower dev_loss but match the adapter-off baseline on every decision metric.
- Adopted from Codex's `2026-10-16-claude-cpu-advice.md`: the choice scoring is
  teacher-forced (gold consequences precede the choice row) and the longest option wins
  25/32 dev variants, so the current metrics cannot show decision quality. Priority 1 is
  prompt-only evaluation plus trivial baselines; the 8-family anchor stays frozen.
- Pending audit before adoption: `research/round2/curriculum-003-candidate.json`
  (16 families, sha256 98c9929a…5461).
- 17:10 — **Owner decisions.** Weekly target: ≥ 20 % (relative) above the best AI on
  FANToM and on OpenToM, reported separately, with an s1-style recipe (arXiv:2501.19393:
  ~1,000 curated problems with reasoning traces, short SFT, budget forcing). Adapter
  widened by owner authorization to LoRA r8/α16 on attention + MLP of blocks 28–31.
  FANToM/OpenToM are evaluation-only. Order of work in `STEERING.md`.
- Thermal guard now also pauses on a lost power-limit profile (> 140 W) and on the
  `hold` file; `ensure-worker.sh` starts it before the worker.
- 18:55 — **Step 1 (honest measurement) started** (commit 5c6c6af). GPU back
  (`CM_PROB_NONE`, 25 °C, 129.6 W limit enforced); worker idle, queue empty. Added
  `metrics.py` (family aggregation shared by model and baselines), a prompt-only decision
  probe (chat prompt + `{"selected_option_id":"`, no gold text; dev only, `probe_*` keys),
  trivial baselines (longest/shortest/always-A/random) and an `eval` job.
  `Worker.better` selects on probe metrics when present; teacher-forced keys stay
  unprefixed as diagnostics. A unit test reproduces Codex's audit (longest option =
  25/32 dev variants). Worker restarted (`stop`→`run`) to load the code. Queued:
  re-prepare t1/t2 (adds dev probes), then eval of the adapter-off base and the four
  overnight `best.pt`.
- 19:14 — **Honest re-measurement of the overnight runs** (dev anchor, 8 families / 32
  variants; prompt-only probe; full table in `runs/eval-*/eval.json` on the run root):

  | arm | choice acc | family all-correct | counterfactual acc | flip (action cf) | p(correct) |
  | --- | --- | --- | --- | --- | --- |
  | base, adapter off | 0.906 | 0.625 | 0.625 | 0/3 | 0.873 |
  | w1-qv-s17 / s29 | 0.906 / 0.906 | 0.625 / 0.625 | 0.625 / 0.625 | 0/3 / 0/3 | 0.829 / 0.825 |
  | w1-qv-lr2e4-s17 | 0.906 | 0.625 | 0.625 | 0/3 | 0.833 |
  | w1-qkvo-s17 | 0.906 | 0.625 | 0.625 | 0/3 | 0.819 |
  | longest option | 0.781 | 0.500 | 0.500 | 0/3 | — |
  | always A / random | 0.469 / 0.188 | 0.125 / 0.000 | — | — | — |

  Reading: the probe and teacher-forced picks coincide. The base is perfect on clean,
  noise and cosmetic variants and fails exactly the three counterfactuals whose decisive
  fact changes the action: it never changes its answer. No adapter changes a single
  decision, and all of them *lower* the probe confidence in the correct option. The
  overnight training learned format, not decisions; nothing is promoted. The anchor
  cannot separate arms except on counterfactual sensitivity (3 families). That is
  the skill to target, and it is close to what FANToM/OpenToM test (beliefs that update
  with new information). Next: steps 2–3 (benchmark harness, wider adapter); new data
  must make counterfactual action changes the hard core, with lengths balanced.
- Known limitation: the probe is decision-first, while the trained format lists the
  alternatives first. It measures decision preference without the model's own
  reasoning. Scoring after self-generated reasoning is still to do (also needed for
  step 6 budget forcing).
- 23:00 — **Step 2 (benchmark harness) started**, owner OK for FANToM/OpenToM (evaluation
  only). Official repos fetched to `/home/archie/wanda/bench/src` (fantom main zip
  sha256 5f7c4712…, OpenToM main zip 0c625eee…). FANToM data from the official GCS URL,
  sha256 `1d08dfa0…53a4` verified against the hash pinned in its loader: 870 sets,
  12,832 questions. Nothing is in git or `datasets/`.
  - `bench.py`: OpenToM imports the official `build_prompt`, `OpenToMUtils` and
    `OpenToMEvaluator` and reproduces `run_baseline.py` sampling (seed 42, 5 batches ×
    50 narratives, coarse location, normal-length narratives, system prompt "You are an
    expert in modeling other's mental state."). FANToM ports `setup_fantom` (short
    input, set aggregation, `random.seed(99)` option order) and `score_and_analyze`;
    sklearn/pandas/evaluate are not installed, so macro/weighted F1 and set
    aggregation are reimplemented and unit-tested against reference values.
  - Generation runs as a worker `bench` job through an owned loopback `llama-server`, so
    thermal pauses and `hold` apply between requests. Greedy, 256 max tokens (the FANToM
    chat-agent default), **thinking disabled** in this first condition. Resumable
    `responses.jsonl`.
  - Found and fixed: `llama-server`'s default 8 GiB host prompt cache plus 32 checkpoints
    of the hybrid model's state grew to 6.2 GiB RSS and starved the 7.9 GiB guest (the
    worker correctly waited on RAM). Now `--cache-ram 1024 --ctx-checkpoints 4`: steady
    1.83 GiB.
  - Not yet computable: FANToM ALL* and belief "distance" need the official embedder
    `sentence-transformers/all-roberta-large-v1` (~1.4 GB download). They are reported as
    `None` until the owner approves that download. ALL and every other metric are computed.
- **Published "best AI" references** (different harnesses; recorded for the target):
  - OpenToM (Xu et al. 2024, ACL; arXiv:2402.06044, Tables 2–3, macro-F1): best LLM per
    type = GPT-4-Turbo with CoT. Loc-coarse FO 0.942 / SO 0.828, MultiHop FO 0.835 /
    SO 0.755, Attitude 0.580 (vanilla GPT-4: 0.643 / 0.442 / 0.658 / 0.637 / 0.544).
    A +20 % relative target on Attitude means ≥ 0.696. On Loc-coarse FO it would mean
    ≥ 1.13, which is impossible, so per-type targets cap at 1.0 and will be reported
    honestly as such.
  - FANToM (official README leaderboard, short input, set aggregation): ALL* 0.8 and
    ALL 2.0 (GPT-4o-2024-05-13); BeliefQ[Choice] best 79.2 (Claude-3-Opus). Third-party
    (Kim et al. 2025, arXiv:2502.11881, a subset with a different protocol): o1-preview
    44.4 / 66.7 / 63.5. That is not directly comparable to the official harness.
    Search found no newer official leaderboard entry.
- 23:43 — **OpenToM, base Bonsai2 9B** (official protocol, 3,250 questions, thinking off,
  0 % corrupted, 0 truncated; `bench/opentom-base/report.json` on the run root), macro-F1
  mean ± std over 5 batches:

  | type | base 9B | GPT-4 vanilla | best published (GPT-4 CoT) | +20 % target |
  | --- | --- | --- | --- | --- |
  | Loc-coarse FO | 0.413 ± 0.016 | 0.643 | 0.942 | 1.000 (capped) |
  | Loc-coarse SO | 0.292 ± 0.037 | 0.442 | 0.828 | 0.994 |
  | MultiHop FO | 0.363 ± 0.014 | 0.658 | 0.835 | 1.000 (capped) |
  | MultiHop SO | 0.299 ± 0.020 | 0.637 | 0.755 | 0.906 |
  | Attitude | 0.351 ± 0.023 | 0.544 | 0.580 | 0.696 |

  The answers collapse onto defaults: "no" 997 vs "yes" 3, "less full" 963 vs "more full"
  33 and "equally full" 4. The model is not tracking beliefs, and the macro-F1 punishes
  that. **Target status: far off track** on every type (gaps of 0.35–0.6 F1). Next
  measurement: thinking-on and budget forcing (step 6) on the same items; first it must
  be checked that the base reasons at all within the token budget.

## 2026-10-17

- 00:20 — Owner: full main-dev autonomy toward the weekly target (this also counts as the
  OK for the FANToM embedder). Night plan, in GPU order: OpenToM thinking pilot →
  procedural ToM tranche → train g1 → native parity of g1 → OpenToM with g1 → FANToM
  base (resumes from 1,152/12,832 answers already saved).
- **Budget forcing** (`NativeChat.think`, s1 arXiv:2501.19393): reasoning inside the
  model's own `<think>` up to N tokens, then the delimiter is forced and the answer
  decoded greedily; early stops can be extended with "Wait". Unit-tested with a scripted
  server. `bench` jobs take `mode=think`, `think_budget`, `min_think`, `max_waits`.
- **Pilot** (running): OpenToM batch 1, first 20 narratives (260 questions), thinking
  budget 512, answer 32 tokens. It will be compared with the direct answers on the same
  items from `opentom-base`. It is a pilot, not the official 5×50 protocol.
- **FANToM embedder**: `sentence-transformers/all-roberta-large-v1` @ cf74d8ac
  (1.4 GB, CPU), mean pooling + cosine as in the official scorer. A sanity test
  passes. `bench-rescore` recomputes ALL* for saved FANToM responses.
- **Step 4, procedural ToM generator** (`tomgen.py`): object worlds (enter/leave/move,
  witnessed or told; first- and second-order beliefs simulated exactly; fullness,
  accessibility, attitude from stated preference) and chat worlds (join/leave around a
  fact update; knowledge, belief content, who-knows). Our own templates in English and
  Spanish, code-derived traces, labels balanced within 2 per (kind, language). 8 unit
  tests cover the belief logic, balance, grammar and determinism. First tranche
  `tom-gen-001` (seed 20261017): 1,400 problems, 1,282 train / 118 dev split by world,
  sha256 ae3ce50d…; run root only, reproducible with `gen-tom`.
- **Batch prefix extractor** (`native/prefix_batch.cpp`): one model load, a fresh context
  per example with the single-shim settings. `prepare_chat` refuses to run unless two
  probes are bit-identical to `prefix_export`. This removes the ~16 s/example cost
  (model reload plus 5 tokenizer processes) that made thousands of problems impractical.
- Queued **g1**: LoRA q,k,v,o on block 31, lr 2e-4, 2 epochs, procedural + decision
  families. Then `verify_native` (exported GGUF in Prism vs the Torch suffix) and OpenToM
  with the adapter, scored against `opentom-base`. Blocks 28–30 are Gated DeltaNet
  recurrent layers; the wider adapter (step 3) needs a faithful differentiable
  implementation with native parity first, so it is not part of tonight's runs.
- 01:13 — **Thinking pilot (OpenToM batch 1, first 20 narratives, 260 questions; base,
  budget 512)** vs the direct answers on the same items, macro-F1:

  | type | direct | think-512 |
  | --- | --- | --- |
  | Loc-coarse FO | 0.444 | 0.444 |
  | Loc-coarse SO | 0.400 | 0.544 |
  | MultiHop FO | 0.419 | 0.597 |
  | MultiHop SO | 0.256 | 0.417 |
  | Attitude | 0.469 | 0.477 |

  Reasoning helps where tracking state matters (multi-hop, second order). First-order
  location stays at the "no" default (73 no vs 7 yes). 182/260 (70 %) hit the 512 budget
  (mean 483 thinking tokens), so a 1024 pilot is queued. It is a pilot subset, not the
  official 5×50 protocol.
- 01:13 — Batch extractor gate **passed** (bit-identical to `prefix_export` on 2 probes);
  `tom-gen-001` prepared in minutes (1,400/1,400, 0 rejected). g1 training started
  (baseline dev loss 2.419 on 150 dev examples).
- Social context pack (owner direction): design note
  `2026-10-17-wanda-social-context-pack-design.md`. Harness `longchat.py`: 20 long chats,
  ≈84k characters (≈24k tokens) each, 232 balanced questions (knowledge 58/58 yes/no,
  belief content) asked over the raw transcript and over the compacted pack (≈2.8k
  tokens, 8.5× shorter). Queued after the 1024 pilot.
- ~02:55 — **g1 finished** (LoRA q,k,v,o block 31, lr 2e-4, 2 epochs). Procedural dev loss
  fell from 2.419 to ≈0.65 (in-distribution: shared templates, not evidence of ToM). The
  decision anchor was unchanged throughout: probe accuracy 0.906, flip 0/3, p(correct)
  0.87–0.88.
- **Native parity of the trained adapter: PASSED** (`verify/g1-qkvo-s17/receipt.json`).
  The exported GGUF in Prism vs the Torch suffix with the same LoRA gives mean KL 4.2e-5
  (max 1.9e-4) and 100 % greedy agreement on 3 public sequences; the layer-30 capture is
  unchanged; the adapter's own effect is KL 0.92, ≈22,000× the parity error. This
  confirms the export basis (LoRA on the pre-Hadamard activation) and that trained
  adapters can be served natively. Not promoted.
- Research note (owner asked how counterfactual flips are best taught): minimal
  counterfactual pairs (Kaushik et al. 2020; edit minimality predicts gains),
  program-generated adversarial ToM data (ExploreToM, ICLR 2025: +27 on ToMi; state
  tracking is the bottleneck) and preference objectives on counterfactual pairs with
  shortcut negatives (C2PO-style). Planned for g2: paired worlds in `tomgen`, weighted
  decision tokens, a DPO-style answer margin against the adapter-off base, and explicit
  longest-option / "no"-default negatives, measured on held-out pairs.
- 03:20 — **OpenToM with g1, direct mode (partial: 1,637 shared questions)**. g1 learned
  the procedural answer format, and in direct mode (thinking off) it often emits a bare
  "Answer:" or no label at all. The official evaluator computes F1 only over parseable
  answers, so its numbers would flatter g1 (Loc-FO 0.645 on 14 % of the answers). Added a
  **strict** macro-F1 (ours, not official; unparseable = wrong, macro over gold labels):

  | type | base | g1 strict | g1 unparseable |
  | --- | --- | --- | --- |
  | Loc-coarse FO | 0.409 | 0.273 | 86 % |
  | Loc-coarse SO | 0.284 | 0.416 | 22 % |
  | MultiHop FO | 0.368 | 0.464 | 10 % |
  | MultiHop SO | 0.295 | 0.359 | 17 % |
  | Attitude | 0.324 | 0.462 | 0 % |

  Real gains on Attitude (+0.14, no format issue), second order and multi-hop. The
  first-order regression is format breakage from training inside `<think>` and
  evaluating without it. Queued next: g1 with thinking (budget 512) on the same 20-narrative
  pilot as the base. Fix for g2: also train direct-answer targets, so the adapter answers
  in the benchmark's format in both modes.
- 03:45 — **g1 complete results.** Direct, full official item set (3,250), strict macro-F1,
  base → g1: Loc-FO 0.413 → 0.233 (89 % unparseable), Loc-SO 0.292 → 0.430, MultiHop FO
  0.363 → 0.475, MultiHop SO 0.299 → 0.371, Attitude 0.351 → 0.425 (0.4 % unparseable).
  **With thinking** (same 260-question pilot, budget 512), base → g1: Loc-FO 0.444 → 0.333,
  Loc-SO 0.544 → 0.425, MultiHop FO 0.597 → 0.547, MultiHop SO 0.417 → 0.464, Attitude
  0.477 → 0.463. **g1 with thinking is worse than the base with thinking.** It reasons in
  ≈48 tokens instead of the base's ≈480, copying our short templated traces, and it
  hallucinates generator details ("Maya was not in the story": a `tomgen` name, absent
  from OpenToM, in 9 of 260 answers). The short synthetic traces replaced the model's
  own reasoning.
- **Plan change (main dev).** g2 → **direct-only**: minimal pairs plus answer weight 1.0
  and pairs kept in one optimizer step, bare labels, tranche `tom-gen-003` (1,600; sha
  f36cc27b…). This tests the decision objective without touching the reasoning style. The think
  path will be trained on **the model's own correct traces** (STaR/s1-style rejection
  sampling: the base reasons on procedural problems with budget forcing, only traces that
  reach the simulated label are kept). That is g3. Generator names must also widen,
  so that no name becomes a learned prior.
- Bug fixed: `minimal_pairs` iterated a set intersection (order depends on
  PYTHONHASHSEED), so `generate_v2` was not reproducible across processes (tom-gen-002
  is affected and stays unused). Now sorted; a cross-process test passes, and tom-gen-003
  is byte-identical under two hash seeds.
- 03:50 — Thermal guard relaxed at the owner's request ("temps are fine"): pause 78 °C /
  resume 68 °C / 20 s rest (was 75/60/45). Over the previous 2 h: p50 54 °C, p95 79 °C,
  max 85 °C, 266 hot pauses. Rollback criteria are in STEERING.md.
- 06:10 — **g2 (direct-only, minimal pairs, answer weight 1.0, pairs per step)** finished at
  dev loss 0.731; the decision anchor was unchanged. **OpenToM direct, partial (1,331
  shared questions)**, strict macro-F1 (identical to official for g2: 0 % unparseable):

  | type | base | g1 strict | g2 |
  | --- | --- | --- | --- |
  | Loc-coarse FO | 0.391 | 0.273 | 0.488 |
  | Loc-coarse SO | 0.249 | 0.432 | 0.344 |
  | MultiHop FO | 0.352 | 0.414 | 0.405 |
  | MultiHop SO | 0.310 | 0.334 | 0.343 |
  | Attitude | 0.354 | 0.609 | 0.374 |

  g2 fixes the format (no unparseable answers) and beats the base on every type; g1 is
  still stronger on attitude and second-order location when it answers. The "no" default
  is weaker but still dominant (388 no vs 24 yes in g2). Full 3,250 and the g2-think pilot
  are next. Partial numbers; final ones replace these.
- 06:28 — **g2 final, OpenToM direct, full official protocol (3,250)**, macro-F1 (official =
  strict, 0 % unparseable), base → g2: Loc-FO 0.413 → 0.428, Loc-SO 0.292 → 0.403,
  MultiHop FO 0.363 → 0.453, MultiHop SO 0.299 → 0.338, Attitude 0.351 → 0.394. This is
  the first adapter that improves every type without breaking the format. g1 (strict,
  same items) still scores higher on Loc-SO (0.430), MultiHop (0.475 / 0.371) and
  Attitude (0.425), but fails the format on Loc-FO (0.233). The "no" default softens (997
  → 939 "no"; "yes" 0 → 61). Target status: still far from GPT-4 CoT (0.94 / 0.83 /
  0.84 / 0.76 / 0.58). The g2-think pilot is running; seed 29 is needed before any
  claim of repeatability.
- 06:58 — **g2 with thinking** (same 260-question pilot, budget 512), base-think → g2-think:
  Loc-FO 0.444 → 0.562, Loc-SO 0.544 → 0.525, MultiHop FO 0.597 → 0.578, MultiHop SO
  0.417 → 0.472, Attitude 0.477 → 0.508; 0 % unparseable. Mean thinking 482 tokens
  (171/260 hit the budget), i.e. the base's own reasoning is preserved, unlike g1
  (48 tokens). Direct-only training combines with test-time reasoning; **g2 + thinking is
  the best configuration so far**. Pilot size: the two small decreases are within noise.
  Queued: the base 1024 pilot, long context, the g2 seed-29 replication, then FANToM.
- 07:45 — **Base, thinking budget 1024 vs 512** (same 260-question pilot): Loc-FO 0.444 →
  0.657, Loc-SO 0.544 → 0.525, MultiHop FO 0.597 → 0.628, MultiHop SO 0.417 → 0.492,
  Attitude 0.477 → 0.477. Mean 732 thinking tokens; 99/260 still hit the budget. A larger
  budget is a strong lever; g2 + 1024 is queued.
- 07:50 — **Long context, base, direct mode** (20 worlds, 232 questions, balanced):
  raw transcript (≈31k tokens): knowledge 0.50 (chance), belief 0.18 (≈ 1/5 chance);
  pack (≈3.9k tokens): knowledge 0.18 and belief 0.11. In the pack the model starts
  reasoning in prose and is cut at 32 tokens (77 truncated), so the pack number is a
  harness artifact, not evidence against compaction. Neither form works without
  reasoning. **KV reuse confirmed**: the first question prefills 31,204 tokens in 16.0 s
  (1,951 tok/s); later questions on the same chat reprocess ≈515 tokens (≈0.5 s). Re-run
  queued with thinking (budget 512) for both forms.
- Thermals with 78/68: p50 66 °C, p95 80 °C, max 85 °C over 2 h. Within the rollback
  criteria.

### Daily summary — 2026-10-17 09:15

- **Plan step reached:** steps 1–2 done (honest measurement; FANToM/OpenToM harness with
  official protocols, strict F1 as an extra), step 4 started (procedural generator,
  minimal pairs), step 6 started (budget forcing works). Step 3 (blocks 28–31) is not
  started: blocks 28–30 are Gated DeltaNet and need a faithful differentiable port.
- **OpenToM, official protocol, best adapter so far (g2 direct, seed 17)** vs base:
  Loc-FO 0.428 (0.413), Loc-SO 0.403 (0.292), MultiHop FO 0.453 (0.363), MultiHop SO 0.338
  (0.299), Attitude 0.394 (0.351). Pilot (260 questions) **g2 + thinking 512**: 0.562 /
  0.525 / 0.578 / 0.472 / 0.508. Base + thinking 1024: 0.657 / 0.525 / 0.628 / 0.492 /
  0.477. g2 + thinking 1024 is queued.
- **FANToM:** base run paused at 1,152/12,832 answers to give the GPU to training; not
  yet scored.
- **Versus the published best (GPT-4 CoT: 0.942 / 0.828 / 0.835 / 0.755 / 0.580) and the
  +20 % target (≈1.0 / 0.99 / 1.0 / 0.91 / 0.70): not on track.** The best pilot numbers
  reach 50–80 % of GPT-4 CoT. Reasoning budget and direct training both add up; the
  realistic next gains come from training on the model's own correct reasoning traces
  (g3) and from a larger budget.
- **Open problems:** "no" default on Loc-FO; g1-style short traces destroy reasoning;
  long-context ledger tracking is at chance without reasoning (re-run with thinking
  queued); the seed-29 replication of g2 is training now.
- 10:30 — **Long context, pack + thinking 512 (partial, 124/232):** knowledge 0.45 (≈ chance,
  mostly "yes"), belief 0.08 (answers the current value instead of the person's belief).
  Every item hit the budget. The 9B cannot derive per-person beliefs from a ~60-event
  ledger even with reasoning. **Design consequence for Cortex:** the pack must carry the
  bookkeeping already done, i.e. each person's last-seen value per fact computed
  deterministically from presence (IRIS/logs), so Wanda reasons on top of it instead of
  recomputing it. Added `pack2` (per-person state table + ledger + recent window; a unit
  test checks it against the simulation) and queued it in direct mode after the
  raw-with-thinking run. g2 seed 29 finished training (dev 0.718); its OpenToM run follows.
- 11:40 — **Long context, complete (base 9B, 20 worlds, 232 balanced questions):**

  | form | mode | knowledge | belief |
  | --- | --- | --- | --- |
  | raw transcript (≈31k tokens) | direct | 0.50 | 0.18 |
  | raw transcript | thinking 512 | 0.51 | 0.16 |
  | ledger pack (≈3.9k) | thinking 512 | 0.47 | 0.11 |
  | **pack2: per-person state + ledger** | direct | **0.70** | **0.36** |

  Chance is 0.50 for knowledge and ≈0.20 for belief. Only the materialised per-person state
  moves the model off chance; 28 of its answers were truncated mid-explanation, so the
  number is a lower bound. This supports the design split: host bookkeeping in Cortex,
  interpretation in Wanda. Queued: pack2 with thinking 512.
- 12:10 — **g2 seed-29 replication, OpenToM direct, full protocol (3,250, 0 % unparseable)**,
  base / g2-s17 / g2-s29: Loc-FO 0.413 / 0.428 / 0.428, Loc-SO 0.292 / 0.403 / 0.362,
  MultiHop FO 0.363 / 0.453 / 0.468, MultiHop SO 0.299 / 0.338 / 0.342, Attitude
  0.351 / 0.394 / 0.398. **The improvement over the base repeats on both seeds for every
  type** (the Loc-SO gain is smaller on s29). g2's recipe (direct-only minimal pairs +
  answer weight) is the current baseline for further training.
- 12:45 — **g3 started (s1/STaR-style self-traces).** New `star` worker job: the base model
  reasons with budget forcing (640 tokens) on owned procedural problems (tom-gen-004, seed
  404, 1,000 problems, sha 376fce42…; 700 of its 916 train-split items are attempted).
  A trace is kept only if it ended on its own and its answer equals the simulated label.
  The kept traces become think-mode targets in the model's own style (g1 showed that short
  templated traces collapse its reasoning to ~48 tokens). g3 then trains on them plus g2's
  data (same r8/α16 q,k,v,o, seed 17) and runs the think-512 OpenToM pilot against g2.
  **Guard race fixed:** at 79–83 °C the guard flips pause/run every few seconds, and one flip
  overwrote a restart's `stop`. `set_cmd` now re-reads control and exits on `stop` instead
  of overwriting it, and the restart helper re-asserts `stop` until the worker exits.
- 12:58 — **OpenToM pilot (260 questions), thinking 1024, base vs g2:** Loc-FO 0.657 / 0.657,
  Loc-SO 0.525 / 0.525, MultiHop FO 0.628 / 0.575, MultiHop SO 0.492 / 0.521, Attitude
  0.477 / 0.508 (0 % unparseable). With long reasoning, g2 is at the base's level (mixed,
  within pilot noise): its direct-mode gain does not add on top of thinking. This is the
  case for g3, which trains the reasoning itself on traces that were checked against the label.
- 13:22–14:45 — The owner used Blender on the GPU (83–85 °C at 100 % by itself); the guard
  kept the worker paused for ~1 h, as intended. **Thermal rollback** (STEERING rule): the last
  2 h reached a sampled max of 89 °C and p95 of 88 °C, mostly Blender, but our run phases add
  on top of it. Guard back to HOT 75 / COOL 60; worker restarted (resumes from saved answers).
