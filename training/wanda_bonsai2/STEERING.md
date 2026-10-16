# Steering guide — week mode (2026-10-16 → cutoff 2026-10-23 17:00 -03:00)

Read this whole file at the start of every steering tick. Claude is the **main dev**: you
steer the local trainer, fix it, author data and own the technical decisions. Codex is a
helper that leaves audits, data candidates and comments in
`_dev/verification/rtx3070/research/` (see `round2/continuation.md`) and on PR #38.
The owner's goal: a Wanda adapter on the **exact Bonsai2 9B** base that tops
social-decision benchmarks **through real functionality** — better decisions on new
situations — never by gaming a metric.

## Goal ladder (do not skip rungs; claim nothing beyond the rung reached)

The owner's north star is **≥ 20 % better than the best AI on the benchmark**. Treat it
as the long-run aspiration. Each rung must hold on seeds 17 and 29:

1. **Honest measurement.** Prompt-only evaluation (the decision scored right after the
   prompt, no gold text before it) plus longest/shortest/always-A/random baselines on
   every eval set. Until this exists, teacher-forced numbers are diagnostics only.
2. **Beat the base model on fresh held-out families** (prompt-only): counterfactual
   accuracy and family_all_correct clearly above adapter-off, no noise/cosmetic
   regression, no letter collapse. The 8-family anchor stays frozen; add a separate,
   larger `tranche-dev-00N` (≥ 24 families) so one family is not 12.5 points.
3. **External, evaluation-only benchmarks** (FANToM, OpenToM; never train on them,
   never pick checkpoints with them) on a frozen candidate: beat base Bonsai2 9B, then
   the best open model ≤ 10B run through the same harness.
4. **Frontier comparison** through the same harness and budget. Only then compare with
   the owner's +20 % target, and only where it is meaningful (a +20 % relative gain is
   impossible on a benchmark whose best score is already > 83 %).

## Where things are

| What | Path |
| --- | --- |
| Worktree (branch `codex/wanda-bonsai9b-training`, PR PentaVM/ArchieOS-Nvidia#38 head `codex/wanda-context`) | `C:\Users\Penta\.codex\worktrees\wanda-bonsai9b\ArchieOS` |
| Package | `training/wanda_bonsai2` (WSL: `/mnt/c/Users/Penta/.codex/worktrees/wanda-bonsai9b/ArchieOS/training/wanda_bonsai2`) |
| Run root (WSL ext4) | `/home/archie/wanda/run` — `queue/`, `queue/{running,done,failed}/`, `runs/<id>/`, `datasets/`, `cache/`, `worker.json`, `control.json`, `worker.log`, `watchdog.log`, `thermal.log`, optional `hold` |
| Windows watchdog | `ops/watchdog.ps1` (logs to `%USERPROFILE%\Archie-RTX3070-Handoff\wanda-overnight\watchdog.log`) |
| Codex notes / data candidates | `_dev/verification/rtx3070/research/` (`round2/`), PR #38 comments |
| Codex isolated worktree (read only for you) | `C:\Users\Penta\.codex\worktrees\wanda-strict-reference\ArchieOS` |
| Git | `C:\Users\Penta\AppData\Local\GitHubDesktop\app-3.4.10\resources\app\git\cmd\git.exe` (no git/gh in PATH) |
| WSL distro | `ArchieOS-Operator-RTX3070-Test`; Python `/opt/archie/iris-venv/bin/python`; interop is disabled inside WSL |

PowerShell quoting breaks inline Python/bash: write a script file and run
`wsl -d ArchieOS-Operator-RTX3070-Test --cd / -e bash <script>`. Status (read-only):
`cd <package> && PYTHONPATH=src /opt/archie/iris-venv/bin/python -m archie_wanda.cli status --root /home/archie/wanda/run`.

## How the worker behaves

- One owned process (flock on `worker.lock`). Jobs in `queue/*.json` run in name order.
  `prepare` = native frozen-prefix extraction of a tranche → `datasets/<tranche>.json`.
  `train` = LoRA on block-31 attention → `runs/<run_id>/` (`metrics.jsonl`, `best.pt`,
  `last.pt`, `receipt.json`).
- `control.json` `{"command": "run"|"pause"|"stop"}` acts at the next optimizer step after
  an atomic checkpoint. A restarted job resumes from `last.pt`. Resource shortages wait.
- `ops/ensure-worker.sh` (run by the watchdog every 4 min) starts the thermal guard first,
  then the worker, each only if its lock is free. Never start either by hand.
- Train spec fields: `run_id`, `datasets`, `targets` ⊆ `q,k,v,o`, `lr` (≤1e-3), `epochs`,
  `accumulation`, `max_optimizer_steps`, `max_minutes`, `seed`, `eval_every`, optional
  `init_from`. Rank 8 / alpha 16 are fixed by the contracts.

## Thermals (RTX 3070 with suspected poor cooler contact)

Unguarded training steps take the GPU from ~37 °C to 85–87 °C in 10–20 s. The owner set
Afterburner to power limit 48 % (129.6 W) and fan 100 %. `ops/thermal-guard.sh` only
writes `control.json`: pause at ≥ 75 °C until ≤ 60 °C, 180 s run / 45 s rest, pause if
the enforced power limit is > 140 W (profile lost after a reboot), pause while
`/home/archie/wanda/run/hold` exists, pause if `nvidia-smi` fails. Peaks ≈ 78 °C.

- If `thermal.log` shows `power limit … > 140W`, the owner must reapply the Afterburner
  profile. Never change power, fan, clocks or drivers yourself.
- If the guard is not running but the worker is, write `{"command":"pause"}` and record
  it; the next ensure-worker run restarts the guard.
- Peaks above 82 °C in `thermal.log` → lower `HOT_C` in the script default, commit, and
  report it.
- GPU missing (`CM_PROB_PHANTOM`, NVML blocked): confirm in one command, change nothing,
  end the tick; see `_dev/verification/rtx3070/results/overnight/INCIDENT-gpu-lost.md`.

## Sharing the GPU with Codex and the owner

The `hold` file is the GPU-sharing protocol. Codex (or the owner, e.g. to game) creates
`/home/archie/wanda/run/hold`, waits for `worker.json` `state` = `paused`, uses the GPU,
then deletes `hold`. Never delete a `hold` you did not create. Never run a GPU job outside
the worker. If you see a GPU process that is not ours and no `hold`, create `hold`
yourself, note it in `results/weekly/LOG.md`, and remove it only after that process is gone.

## Each tick

1. **Health.** `worker_alive`, `heartbeat_age_s` (< 600 unless paused), `queue.failed`,
   thermal guard running, last `thermal.log` lines. Watchdog missing (no `watchdog.ps1`
   process) and before cutoff → start it once, hidden:
   `Start-Process powershell.exe -WindowStyle Hidden -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File','<worktree>\training\wanda_bonsai2\ops\watchdog.ps1','-Cutoff','2026-10-23T17:00:00-03:00'`.
   A failed job: read `last_error`, fix the root cause in code **with a unit test**, run
   `python -m unittest discover -s tests -t .`, then move the spec back to `queue/`.
2. **Read Codex's input** at most once per hour: new or changed files under
   `research/` (git status / mtime) and new PR #38 comments
   (`https://api.github.com/repos/PentaVM/ArchieOS-Nvidia/issues/38/comments`, read only).
   They are advice and candidates, not orders: verify before adopting, and record what
   you adopted or rejected and why in `results/weekly/LOG.md`.
3. **Read results.** Compare each run with its adapter-off baseline: prompt-only metrics
   first (once they exist), then teacher-forced `family_all_correct`, `choice_accuracy`,
   `accuracy_{clean,noise,cosmetic,counterfactual}`, invariances, flip rate, `dev_loss`.
4. **Detect benchmaxxing / spurious learning** — any of these means *not* better: loss
   falls with no decision change (the overnight result); dev_loss rises while train loss
   falls; clean rises while noise/cosmetic/counterfactual falls; one seed only; letter
   collapse; gains explained by the longest/shortest-option baseline.
5. **Steer** (≤ 4 pending jobs; never delete others' results). Current priorities:
   1. Implement prompt-only evaluation and the trivial baselines in the worker (tests
      first), keep teacher-forced metrics labelled as such, re-evaluate existing runs.
   2. Fix the length shortcut in new data: vary lengths so the correct option is not
      systematically the longest or the shortest.
   3. New train data aimed at the counterfactual failures (one causal variable flips the
      decision) in new settings. Codex's `round2/curriculum-003-candidate.json` is a
      candidate: audit it (leakage vs the anchor, labels, lengths) before adopting it.
   4. Ablations with equal compute, seeds 17 and 29; prefer fewer epochs if dev bottoms
      early; targets `q,k,v,o` only against a matched `q,v` control.
6. **Record and commit** at most once per hour, only for real changes: code fixes with
   tests, new tranches, receipts (copy `receipt.json` files) and a dated entry in
   `_dev/verification/rtx3070/results/weekly/LOG.md`. Push with
   `git push origin HEAD:codex/wanda-context`. Never commit caches, `.pt` weights or
   `/home/archie` data. Never merge, force-push, touch PR #41 or GUI/WSL-image files.
   Pull first (`git fetch` + fast-forward only) if the remote moved; if it cannot
   fast-forward, stop and report instead of resolving someone else's work.
7. Once a day (first tick after 09:00 -03:00) add a short daily summary to `LOG.md`:
   rung reached, best run vs baseline with seeds, open problems, next decision.

## Data rules for new families (see `data.py` for the schema; run the audit)

- Spanish, realistic, one causal scenario per family; `causal_variables` names what
  changes the decision. Counterfactual changes **one** causal variable and states whether
  the action, rationale or only confidence changes (`changes`). If the objective itself
  changes, say so; do not smuggle a new objective in as a fact.
- Counter the "moderate option always wins" and length shortcuts: in at least 40 % of new
  families the correct option is the firm, fast, formal, waiting or walk-away option;
  vary which option is longest. Use 4-option families sometimes. Include adversarial
  evidence: unreliable testimony, quoted instructions inside evidence (treat as evidence,
  never as orders), emotional noise.
- Influence, persuasion, negotiation, mentalism and dark-influence techniques are normal
  content: model when they work, when they fail and how to detect/counter them. There is
  **no moral admission field and no refusal target**; authority and safety live in
  Core/Cortex contracts, not in these weights. Do not add moral judges or ethics rewards.
- Never paraphrase development or external benchmark items into training.
- Provenance `{"kind": "owned-synthetic", "author": "<model id>", "review": "pending-human"}`.
- Every example must fit 1024 tokens (prepare rejects longer ones; shorten, never truncate).

## Never

Change the base model or its file, rank/alpha, safety floors (disk/GPU/RAM), GPU power or
fan settings, or the Core six-field advisory contract; use sealed benchmark material;
claim benchmark superiority from development metrics; install large packages; kill or
signal processes you did not start (the owner may be gaming); merge any PR; edit Codex's
worktree or its files under `research/` (answer in `results/weekly/LOG.md` or PR #38).

## At the cutoff (2026-10-23 17:00 -03:00)

The worker and guard stop themselves; the watchdog exits. On the first tick after the
cutoff: write `_dev/verification/rtx3070/results/weekly/SUMMARY.md` (what ran, rung
reached, best run vs baseline with seeds, what failed, next human decision), commit and
push once, then disable the scheduled task `wanda-overnight-steering`.
