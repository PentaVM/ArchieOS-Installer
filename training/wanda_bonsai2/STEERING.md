# Steering guide — week mode (2026-10-16 → cutoff 2026-10-23 17:00 -03:00)

Read this whole file at the start of every steering tick. Claude is the **main dev**: you
steer the local trainer, fix it, author data and own the technical decisions. Codex is a
helper that leaves audits, data candidates and comments in
`_dev/verification/rtx3070/research/` (see `round2/continuation.md`) and on PR #38.
The owner's goal: a Wanda adapter on the **exact Bonsai2 9B** base that tops
social-decision benchmarks **through real functionality** — better decisions on new
situations — never by gaming a metric.

## Weekly target (owner decision, 2026-10-16)

**By 2026-10-23: score ≥ 20 % (relative) above the best AI on FANToM and on OpenToM,
each reported separately**, with a rushed-prototype recipe modelled on s1
(arXiv:2501.19393: 1,000 curated questions with reasoning traces chosen for quality,
difficulty and diversity, SFT, then *budget forcing* at test time — end the thinking or
extend it with "Wait"). The "best AI" reference is the highest published score for each
benchmark metric; record the source, date and metric definition in `LOG.md`, and say
plainly that published numbers come from a different harness. Both benchmarks are
**evaluation only**: never train on them, never paraphrase their items, never select
checkpoints or tune hyperparameters on them. Keep a frozen candidate for the final run.

Report honestly where we stand against the target every day; a miss is a result, not a
reason to soften the measurement. Order of work (each step must hold on seeds 17 and 29):

1. **Honest measurement.** Prompt-only evaluation (the decision scored after the prompt
   and the model's own reasoning, no gold text before it) plus longest/shortest/always-A/
   random baselines on every eval set. Teacher-forced numbers stay labelled diagnostics.
2. **Benchmark harness.** Loaders and scorers for FANToM and OpenToM following each
   paper's official metrics (FANToM All*/All plus per-question-type; OpenToM macro-F1
   per question type, grouped by narrative). Data from the official GitHub repos only,
   stored under `/home/archie/wanda/bench/` (never in git, never in `datasets/`).
   Measure base Bonsai2 9B first: that is the starting point.
3. **Wider adapter.** Owner authorized: LoRA rank 8 / alpha 16 on attention *and* MLP of
   blocks 28–31 (frozen native prefix through block 27). Change contracts and extraction
   with unit tests and a numerical parity check against the current block-31 path;
   keep the old path working for reproducing earlier receipts.
4. **s1K-social data.** Build a pool of several thousand owned-synthetic decision
   problems, each target = short reasoning trace (evidence → who knows/believes what →
   objectives → consequences of each option → decision), then select 1,000 by
   **quality** (audit passes, no length shortcut, labels reviewed), **difficulty**
   (base model wrong or low-margin under prompt-only scoring) and **diversity**
   (mechanism × domain × format coverage). Cover the skills the benchmarks test —
   information asymmetry, people entering/leaving a conversation, first/second-order
   belief, attitudes, multi-hop — in our own settings and formats, never their items.
   Traces must fit the 1024-token limit.
5. **Train** the s1K-social set on the wider adapter (few epochs; equal-compute ablation
   against the old data; seeds 17/29). Select checkpoints only on our own dev sets.
6. **Budget forcing** in generation: a thinking budget, forced end, and "Wait"
   extension; sweep budgets on our dev sets, then freeze one setting.
7. **Final:** frozen candidate + frozen budget on FANToM and OpenToM, once, against the
   published best. The 8-family anchor stays frozen throughout.

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
  `train` = LoRA on the adapted blocks → `runs/<run_id>/` (`metrics.jsonl`, `best.pt`,
  `last.pt`, `receipt.json`).
- `control.json` `{"command": "run"|"pause"|"stop"}` acts at the next optimizer step after
  an atomic checkpoint. A restarted job resumes from `last.pt`. Resource shortages wait.
- `ops/ensure-worker.sh` (run by the watchdog every 4 min) starts the thermal guard first,
  then the worker, each only if its lock is free. Never start either by hand.
- Train spec fields: `run_id`, `datasets`, `targets` ⊆ `q,k,v,o`, `lr` (≤1e-3), `epochs`,
  `accumulation`, `max_optimizer_steps`, `max_minutes`, `seed`, `eval_every`, optional
  `init_from`. Rank 8 / alpha 16 are fixed by the contracts. Until step 3 of the weekly
  target lands, the adapter is block-31 attention only; afterwards blocks 28–31
  attention + MLP (owner authorization, 2026-10-16).

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
5. **Steer** (≤ 4 pending jobs; never delete others' results). Follow the order of work
   in "Weekly target". Steps 1–3 are code; step 4 (data) can be authored in parallel on
   any tick that has spare time. Notes:
   - Fix the length shortcut in new data: vary lengths so the correct option is not
     systematically the longest or the shortest.
   - Codex's `round2/curriculum-003-candidate.json` is a candidate for the pool: audit it
     (leakage vs the anchor and the benchmarks, labels, lengths) before adopting it.
   - Ablations with equal compute, seeds 17 and 29; prefer fewer epochs if dev bottoms
     early.
   - GPU time is the scarce resource (thermal duty cycle): queue evaluation and training
     so the GPU is rarely idle, but never at the cost of the thermal rules.
6. **Record and commit** at most once per hour, only for real changes: code fixes with
   tests, new tranches, receipts (copy `receipt.json` files) and a dated entry in
   `_dev/verification/rtx3070/results/weekly/LOG.md`. Push with
   `git push origin HEAD:codex/wanda-context`. Never commit caches, `.pt` weights or
   `/home/archie` data. Never merge, force-push, touch PR #41 or GUI/WSL-image files.
   Pull first (`git fetch` + fast-forward only) if the remote moved; if it cannot
   fast-forward, stop and report instead of resolving someone else's work.
7. Once a day (first tick after 09:00 -03:00) add a short daily summary to `LOG.md`:
   step of the weekly plan reached, current FANToM/OpenToM numbers (once measured) vs
   base and vs the published best, best run vs baseline with seeds, open problems, and
   whether the +20 % target is on track (say so plainly if it is not).

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

Change the base model or its file, rank/alpha, the adapted blocks beyond 28–31,
safety floors (disk/GPU/RAM), GPU power or fan settings, or the Core six-field advisory
contract; train on, paraphrase or tune against FANToM/OpenToM or other sealed material;
claim benchmark superiority from development metrics; install large packages; kill or
signal processes you did not start (the owner may be gaming); merge any PR; edit Codex's
worktree or its files under `research/` (answer in `results/weekly/LOG.md` or PR #38).

## At the cutoff (2026-10-23 17:00 -03:00)

The worker and guard stop themselves; the watchdog exits. On the first tick after the
cutoff: write `_dev/verification/rtx3070/results/weekly/SUMMARY.md` (what ran, final
FANToM/OpenToM results vs base and vs the published best — target met or missed —, best
run vs baseline with seeds, what failed, next human decision), commit and
push once, then disable the scheduled task `wanda-overnight-steering`.
