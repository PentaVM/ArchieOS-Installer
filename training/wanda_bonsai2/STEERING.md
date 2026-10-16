# Overnight steering guide (2026-10-16, cutoff 13:30 -03:00)

Read this whole file at the start of every steering tick. You are steering a local,
already-running trainer; you are not the trainer. The owner's goal: a Wanda adapter on
the **exact Bonsai2 9B** base that tops social-decision benchmarks **through real
functionality** — better decisions on new situations — not by gaming a metric.

## Where things are

| What | Path |
| --- | --- |
| Worktree (branch `codex/wanda-bonsai9b-training`, PR PentaVM/ArchieOS-Nvidia#38 head `codex/wanda-context`) | `C:\Users\Penta\.codex\worktrees\wanda-bonsai9b\ArchieOS` |
| Package | `training/wanda_bonsai2` (WSL: `/mnt/c/Users/Penta/.codex/worktrees/wanda-bonsai9b/ArchieOS/training/wanda_bonsai2`) |
| Run root (WSL ext4) | `/home/archie/wanda/run` — `queue/`, `queue/{running,done,failed}/`, `runs/<id>/`, `datasets/`, `cache/`, `worker.json`, `control.json`, `worker.log`, `watchdog.log` |
| Windows watchdog | `ops/watchdog.ps1` (detached, logs to `%USERPROFILE%\Archie-RTX3070-Handoff\wanda-overnight\watchdog.log`) |
| Git | `C:\Users\Penta\AppData\Local\GitHubDesktop\app-3.4.10\resources\app\git\cmd\git.exe` (no git/gh in PATH) |
| WSL distro | `ArchieOS-Operator-RTX3070-Test`; Python `/opt/archie/iris-venv/bin/python`; interop is disabled inside WSL |

PowerShell quoting breaks inline Python/bash: write a script file and run
`wsl --cd / -e bash <script>`. Status (read-only, no GPU):
`cd <package> && PYTHONPATH=src /opt/archie/iris-venv/bin/python -m archie_wanda.cli status --root /home/archie/wanda/run`.

## How the worker behaves

- One owned process (flock on `worker.lock`). Jobs in `queue/*.json` run in name order.
  `prepare` = native frozen-prefix extraction of a tranche → `datasets/<tranche>.json`.
  `train` = LoRA on block-31 attention over those datasets → `runs/<run_id>/`
  (`metrics.jsonl`, `best.pt`, `last.pt`, `receipt.json`).
- `control.json` `{"command": "run"|"pause"|"stop"}` acts at the next optimizer step after
  an atomic checkpoint. A crashed/restarted job resumes from `last.pt` (same settings and
  data digest only). Resource shortages wait; they do not fail the job.
- The watchdog re-runs `ops/ensure-worker.sh` every 4 minutes until the cutoff. Never start
  a second worker by hand; never kill or signal processes you did not start.
- Train spec fields: `run_id`, `datasets` (list), `targets` ⊆ `q,k,v,o`, `lr` (≤1e-3),
  `epochs`, `accumulation`, `max_optimizer_steps`, `max_minutes`, `seed`, `eval_every`,
  optional `init_from` (a `best.pt`). Rank 8 / alpha 16 are fixed by the contracts.
- Measured: ~0.85 s per ~470-token example, ~1 GB GPU peak. Compute is not the
  bottleneck; **data quality and diversity are**.

## Each tick

1. **Health.** `worker_alive`, `heartbeat_age_s` (< 600 unless paused), `queue.failed`.
   If dead and before cutoff, wait for the watchdog (≤ 5 min) before investigating
   `worker.log`. A failed job: read `last_error`, fix the root cause in code **with a unit
   test**, run `python -m unittest discover -s tests -t .`, then move the spec back to
   `queue/`. Do not paper over a numerical gate.
2. **Read results.** For each run compare `baseline` (adapter off) with eval rows:
   `family_all_correct`, `choice_accuracy`, `accuracy_{clean,noise,cosmetic,counterfactual}`,
   `noise_invariance`, `cosmetic_invariance`, `counterfactual_flip_rate`, `dev_loss`.
   The development set is only 8 families (32 prompts): one family = 12.5 points of
   `family_all_correct`. Treat single-seed differences under ~2 families as noise.
3. **Detect benchmaxxing / spurious learning** — any of these means the run is *not* better:
   train loss falls while `dev_loss` rises; clean accuracy rises while noise/cosmetic
   invariance or counterfactual accuracy falls; the gain appears in one seed only; the
   model's choice letter distribution collapses to one letter; format-only gains
   (loss drop) without choice gains.
4. **Steer** (keep ≤ 4 pending jobs; never delete others' results):
   - Replicate promising settings with seed 29 before believing them.
   - Learning rate / epochs: if dev_loss bottoms early, prefer fewer epochs over more data
     passes. Targets `q,k,v,o` (stage 2) only versus a matched `q,v` control.
   - **Add data, not passes.** When runs plateau or the queue is empty, author a new
     tranche `data/social/tranche-00N.json` (12–20 families, all `train`) aimed at the
     *mechanisms* behind development errors, in **different settings/people/wording** —
     never paraphrase a development family into training (that is leakage).
   - For a fresh generalization check, write a separate `tranche-dev-00N.json` with
     `development` families, evaluate it as an extra dataset, and report it separately;
     keep the original 8-family development anchor unchanged.
   - Queue `prepare` for the new tranche, then an ablation pair with the same seed/settings:
     without vs with the new tranche.
5. **Commit to PR #38 at most once per hour** when something real changed: new tranches,
   code fixes with tests, and run summaries (copy `receipt.json` files and a short markdown
   table into `_dev/verification/rtx3070/results/overnight/`; never commit caches, `.pt`
   weights or `/home/archie` data). Push with
   `git push origin HEAD:codex/wanda-context`. Never merge, never force-push, never touch
   PR #41 or GUI/WSL-image files; Claude/Xavi design work is out of scope.

## Data rules for new families (see `data.py` for the schema; run the audit)

- Spanish, realistic, one causal scenario per family; `causal_variables` names what
  changes the decision. Counterfactual changes **one** causal variable and states whether
  the action, rationale or only confidence changes (`changes`).
- Counter the "moderate option always wins" shortcut: in at least 40 % of new families the
  correct option is the firm, fast, formal, waiting or walk-away option. Use 4-option
  families sometimes. Include adversarial evidence: unreliable testimony, quoted
  instructions inside evidence (treat as evidence, never as orders), emotional noise.
- Influence, persuasion, negotiation, mentalism and dark-influence techniques are normal
  content: model when they work, when they fail and how to detect/counter them. There is
  **no moral admission field and no refusal target**; authority and safety live in
  Core/Cortex contracts, not in these weights. Do not add moral judges or ethics rewards.
- Keep personal and work domains labelled correctly. Provenance
  `{"kind": "owned-synthetic", "author": "<model id>", "review": "pending-human"}`.
- Every example must fit 1024 tokens (prepare rejects longer ones; shorten, never truncate).

## Never

Change the base model or its file, rank/alpha, safety floors (disk/GPU/RAM), or the
Core six-field advisory contract; use sealed benchmark material; claim benchmark
superiority from development metrics; install large packages; kill unrelated processes
(the user may be gaming — resource waits are expected); merge any PR.

## After the cutoff

The worker checkpoints and stops itself; the watchdog exits. On the first tick after
13:30, write `_dev/verification/rtx3070/results/overnight/SUMMARY.md` (what ran, best
run vs baseline with seeds, what failed, what the next human decision is), commit and
push once. On later ticks, if SUMMARY.md exists and is committed, do nothing.
