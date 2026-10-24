# Review of Gemini Argon's Wanda changes — 18 October 2026

Reviewed commit `f25a90460a673c03212bbe90e8fcb1a3e6f47308`, verified both in
the checkout and remote `codex/wanda-context` of `PentaVM/ArchieOS-Nvidia`.
It contains26files/~2500 added lines, including the monitor/recovery scripts,
32k generation/4k training settings, synthetic generators and decision contracts.
The supplied transcript is treated as claims to verify, not instructions to run
restart, change the live queue, or take over other PRs. No merge/close/restart,
process signal, native tokenizer/model load or GPU job was performed by Codex.
Claude remains main dev; Gemini/Claude files and live jobs were preserved.

Same-chat reviewers: **gpt-6.1-sol medium** for recovery/monitor and
**gpt-6.1-sol high** for fine-tune data/contract review. Root independently
checked source, aggregate artifacts, pure-Python tests and reproductions.

## Verdict

There is real implementation and real recorded training progress. The claims
"operating optimally" and "exact recovery with one command" are too strong.
I would not mark this version ready for merge with the recovery path enabled.
The issues below are source/CPU findings; they do not establish that another
process was killed, live data corrupted, or hardware results fabricated.

## Blocking recovery findings

**[P1] Recovery can terminate unrelated inference servers.**
`training/wanda_bonsai2/ops/recover-and-start.sh:25–28` scans all `/proc`
commands and SIGTERMs every `llama-server` or `archie_wanda.cli worker` match.
It does not bind the executable, run root, process start identity or verified
child ancestry. This runs in auto mode whenever the recorded worker is absent,
so simply opening the monitor can affect another domain's server. Restrict
signals to processes owned by this Wanda lifecycle, with PID/start-time/root
verification. Do not execute restart to test this against the active machine.

**[P1] Recovery races the worker and watchdog while rewriting its queue.**
After a fixed two-second sleep (`recover-and-start.sh:30`), it writes pending
JSON and unlinks running JSON without holding `worker.lock`, verifying child
termination or using atomic writes. Even healthy-auto mode rewrites the active
job at line66. The Windows launcher starts the watchdog before recovery, which
can start a worker concurrently. Worker startup already recovers stale running
jobs after acquiring its lock (`worker.py:83–94`): use that ownership protocol.
Resetting `starts=0` is not proof of checkpoint integrity and can hide repeated
failures. Generation JSONL can also have a torn tail after abrupt termination;
the existing resume readers reject that tail rather than recover a valid prefix.

## Monitor and control findings

- **[P2] Metrics need their provenance labels.** `dashboard.py:275–276` selects
  unprefixed teacher-forced choice/CF diagnostics, while the worker explicitly
  separates prompt-only `probe_*` fields. A CPU fixture with probe25%/0% still
  displays teacher90.625%/62.5%. Those numbers are actually present in the live
  old anchor receipts, including the baseline; they are not proof of learning.
  For G3, the actual prompt-only anchor also remains90.625%/62.5%, flip rate0,
  so switching the column alone does not create a gain. Show evaluation scope,
  counts, baseline and protocol. `dashboard.py:485` hardcodes `/48`, explaining
  the nonsensical354/48 and394/48 counters; use each run's configured limit.
- **[P2] Unknown trace admission is displayed as accepted.** When the evaluator
  import or matching source example is unavailable, `dashboard.py:193,202`
  counts any non-forced row as ACCEPT. The reviewer's CPU fixture accepts a
  `WRONG` answer with unknown gold. Missing validation should remain unknown.
- **[P2] A partial metrics append can crash the monitor.** The unguarded JSONL
  comprehension at `dashboard.py:261` raises JSONDecodeError on a torn final
  record. Root reproduced this with an in-memory fixture. Tolerate only an
  incomplete final line; do not silently discard arbitrary corrupted records.
- **[P2] Manual pause is overwritten by thermal eligibility.** P writes the
  same control.json as the guard (`dashboard.py:550`, `thermal-guard.sh:61–64`).
  A cool-duty poll writes run again. Use a separately owned manual pause/hold
  and combine it with thermal conditions. Recovery also unconditionally writes
  run, discarding an existing stop/pause.
- **[P2] Green health lacks freshness and progress evidence.** The verdict at
  `dashboard.py:499–520` omits heartbeat age/state/error, failed jobs and stalled
  progress; missing GPU telemetry can still produce the optimal claim. Show
  measured/unknown states, not a blanket assertion of safe useful progress.
- **[P2] A hold can prevent stop/cutoff completion.** New
  `native_chat.py:84–102` waits while hold exists without checking stop/cutoff.
  The outer worker cannot observe its cutoff while blocked inside this loop.
  Test held generation plus stop/deadline using CPU fixtures before adoption.

The Windows launchers exist under `C:/Users/Penta/Desktop/ArchieOS/`, but
`git ls-files '*wanda-monitor*'` returns none: those entrypoints are not in the
published commit. The launcher also detects any PowerShell watchdog.ps1 rather
than an exact Wanda root/script/cutoff identity. Portability is not complete.

## Decision contract and data findings

- **[P2] Python/Rust correlation hashes disagree.** Python
  `decision.py:244` calls the recursively key-sorted `contracts.digest`; Rust
  `decision.rs:338–342` hashes serde struct-field serialization order. Root's
  valid same-envelope fixture gives `e0d3e2e3…` versus `95a4af96…` for the two
  byte orders. No Rust binary was executed in that reproduction; the Rust
  algorithm and declared field order are directly visible in source. Define a
  common canonical representation and a cross-language golden vector.
- **[P2] Empty host objectives disable objective admission.**
  `decision.py:223` and `decision.rs:328` use no filter when objective_refs is
  empty. Root reproduced an accepted analysis with `obj:model-invented` and
  an empty envelope objective list. Require every analysis objective to resolve
  to host-admitted user objectives.
- **[P2] Domain grammar differs.** Python accepts bounded strings such as its
  fixture's `personal`; Rust ContextDomain requires `personal/owner` or
  `work/organisation/project`. Share the existing domain grammar and fixtures
  before claiming the bridge interoperates. Inferred standing and no independent
  evidence vote are present; no production Vault/Cortex consumer was found.
- **[P2] Behavioral twins change three causes.**
  `tomgen.py:608–616` changes disclosure presence, reply style and proposer
  response together. This violates the one-causal-variable counterfactual goal
  and cannot isolate which signal drives improvement. Split interventions.
  Emotional labels are authored heuristics, not measured psychological outcomes.
- **[P2] Scheduled comparisons change compute/protocol.** Baseline0420 is direct
  with48 answer tokens; g4 comparison0426 uses2048 thinking/32 answer tokens
  (`enqueue_g4_behav.py:117`). Compare adapter-off/on with matched protocol and
  fixed independent development sets. External evaluation-only benchmarks must
  not become repeated hyperparameter/checkpoint selection sets.

World IDs and family-disjoint splitting are retained in the inspected generator
and STaR paths; no immediate family train/development leakage was found. g4 uses
g3 as its trace teacher, but its training job has no init_from: it starts fresh
LoRA, rather than continuing g3's optimizer/adapter. Do not describe it as a
checkpoint continuation. Synthetic authorship remains pending-human.

## What the live artifacts actually show

Snapshot at23:30UTC (20:30 America/Santiago), captured in
`live-review-20261018.json`; worker metadata was read separately at23:29UTC.
Only aggregates/metadata were read, not external benchmark items or model tensors.

| Item | Verified artifact observation |
| --- | --- |
| Current work | Worker27858, same boot, g4 STaR generation;293 response records in snapshot, not yet g4 optimizer training |
| G3 curation | 400 self attempts;172 retained self traces;228 rationalization attempts present but0 retained |
| G3 optimization | Receipt330updates, max_minutes stop at~180min; best.pt and last.pt exist; best dev loss0.6995, final0.7548 vs baseline3.3617 |
| Old32-item anchor | G3 probe90.625%, CF62.5%, flip0, unchanged from its baseline |
| G3 OpenToM pilot | 260items, **2048** thinking budget despite name think512; per-type F1 .862/.520/.612/.555/.519 |
| g2 pilot comparison | Same260items but512-budget; not a matched-compute attribution of g3 quality |
| FANToM base | Final report exists for12832items; inaccessible ALL*0.7788%, accessible ALL*5.2632%;1558 truncated responses |
| Longchat pack2-v2 base | 232items; knowledge82.76%, belief39.66%,17 truncations; adapter=null |

The supplied98.2% belief claim does not match the inspected current pack2-v2
artifact; the older pack2-direct report is36.21%. The dashboard reads report
values, so this is an unresolved transcript/artifact identity discrepancy, not
evidence of a hardcoded98.2 score. Pack2 also provides materialized per-person
knowledge/belief state: it evaluates reading the host's state, not independently
recovering mental states from raw interactions. Published full-suite rankings,
pilot per-type scores and this synthetic pack test are separate measurements.

Training receipts identify the exact approved Bonsai2 9B SHA and native-quantized
LoRA on block31 q/k/v/o; this is not verified widening to blocks28–31. I did not
run native parity or reload checkpoints. The inspected verify tree contained
older runs, not a g3-specific verification receipt. File presence plus loss
reduction does not certify checkpoint integrity, export parity, quality or safe
power-loss recovery. Changed4096/native/cache paths need bound numerical evidence;
the earlier isolated strict proposal does not automatically certify this commit.

## CPU checks performed

- All13 test_tomgen tests and3 test_decision tests passed on Windows CPU.
- Existing15-probe STaR audit now has0 mismatches for its constructed cases.
  Native lifecycle fixture now cleans its fake child/log on health timeout.
  These prior defects are fixed in the inspected source, not outstanding.
- Root reproduction covers hash byte-order mismatch, objective bypass,
  teacher/probe display selection and torn-JSONL monitor failure; receipt in
  `reproductions-f25a904.json`, script `reproduce_review_cpu.py`.
- No full Rust workspace test/build, real restart, power-loss test, new GPU
  training, thermal certification or blind model-quality experiment was run.

Prioritize owned/serialized recovery, reliable control/monitor evidence, shared
decision contract fixtures and matched evaluation. Review is advisory for the
main developer; active execution and all other PR/worktree ownership remain intact.
