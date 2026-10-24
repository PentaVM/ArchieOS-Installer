# Overnight Wanda run — summary (2026-10-16, cutoff 13:30 -03:00)

## What ran

| Job | Result |
| --- | --- |
| `001-prepare-t1` | done, 80/80 examples admitted |
| `002-prepare-t2` | done after the GPU incident (resumed from cache at 37/80) |
| `010-train-w1-qv-s17` | done, 48 optimizer steps |
| `011-train-w1-qv-s29` | done, 48 steps |
| `012-train-w1-qkvo-s17` | done, 48 steps |
| `013-train-w1-qv-lr2e4-s17` | done, 48 steps |

No job failed. All train runs: tranche-001 + tranche-002 (128 train examples), 3 epochs,
accumulation 8, rank 8 / alpha 16, eval every 2 steps on the 8-family / 32-prompt
development anchor. Receipts are in
`_dev/verification/rtx3070/research/overnight-receipts/` (committed in b0fb2f9).

## Best run vs baseline

| Run | family_all_correct | choice_accuracy | acc counterfactual | acc clean / noise / cosmetic | mean_p_correct | dev_loss |
| --- | --- | --- | --- | --- | --- | --- |
| baseline (adapter off) | 0.625 | 0.906 | 0.625 | 1.0 / 1.0 / 1.0 | 0.903 | 2.439 |
| w1-qv-s17 | 0.625 | 0.906 | 0.625 | 1.0 / 1.0 / 1.0 | 0.893 | 2.074 |
| w1-qv-s29 | 0.625 | 0.906 | 0.625 | 1.0 / 1.0 / 1.0 | 0.892 | 2.069 |
| w1-qkvo-s17 | 0.625 | 0.906 | 0.625 | 1.0 / 1.0 / 1.0 | 0.887 | 1.799 |
| w1-qv-lr2e4-s17 | 0.625 | 0.906 | 0.625 | 1.0 / 1.0 / 1.0 | 0.902 | 1.691 |

**No run is better than the baseline.** Every run lowers `dev_loss` (best: lr 2e-4,
2.44 → 1.69) but no decision changes: family, choice and counterfactual accuracy are
identical to the adapter-off baseline at every eval, in both seeds. `mean_p_correct` does
not rise (it falls slightly). Per the steering guide this is a format-only gain: the
adapter learns the answer style, not better decisions.

The remaining error is counterfactual: 3 of 8 families fail when one causal variable
changes and the correct decision should flip (`counterfactual_flip_rate` 0.0 for
baseline and all runs).

## Incidents

- **04:30 GPU dropped off the bus** (see `INCIDENT-gpu-lost.md`). The owner rebooted;
  the watchdog was restarted at 09:39 with the original 13:30 cutoff.
- **Thermals.** Training steps took the RTX 3070 from ~37 °C to 85–87 °C within
  10–20 s at only ~130 W, which suggests poor heatsink contact or dried paste. The owner
  set Afterburner to power limit 48 % (129.6 W enforced) and fan 100 %. A new
  `training/wanda_bonsai2/ops/thermal-guard.sh` pauses the worker through `control.json`
  above 75 °C (resume ≤ 60 °C) with a 180 s run / 45 s rest duty cycle. It only acts
  between optimizer steps, so peaks reached ~78 °C. It changes no GPU setting and never
  signals a process; it exited at the cutoff.
- A second session also worked in this worktree: commit b0fb2f9 (13:15) reviewed the
  four receipts. No conflicting worker or GPU job was seen.

## Next human decision

1. **Data, not passes.** Author a tranche aimed at counterfactual sensitivity: one causal
   variable flips the correct option, in new settings and wording (no paraphrase of
   development families), ≥ 40 % firm/fast/formal/waiting/walk-away answers. Then
   queue prepare + a with/without ablation pair on seed 17 and replicate on seed 29.
2. **Hardware.** Check the GPU cooler (paste, contact, dust) before long unattended runs.
   Until then keep the 48 % power limit, fan at 100 %, and `thermal-guard.sh` running.
