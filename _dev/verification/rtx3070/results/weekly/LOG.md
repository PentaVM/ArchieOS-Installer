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
