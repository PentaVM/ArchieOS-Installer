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
- Thermal guard now also pauses on a lost power-limit profile (> 140 W) and on the
  `hold` file; `ensure-worker.sh` starts it before the worker.
