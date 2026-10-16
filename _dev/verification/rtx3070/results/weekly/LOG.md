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
