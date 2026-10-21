# Recovery and STaR follow-up — 2026-10-18 05:25 UTC

Claude remains main developer. Origin verified `PentaVM/ArchieOS-Nvidia`,
HEAD6660aec plus Claude's concurrent dirty changes. A same-chat **gpt-6.1-sol
medium** reviewer inspected resource accounting read-only; root independently
ran CPU acceptance and lifecycle fixtures. No live code, queue, control, hold,
locks, thermal settings or watchdog were changed. No native process, model,
tokenizer or GPU workload was launched by Codex.

## Recovery observed, with a new resource wait

The earlier absence is historical: at05:21:52UTC, workerPID14364 existed in the
current WSL boot, with an80-second-old heartbeat. Its command identifies the
documented Wanda CLI worker; guard and ensure-worker commands also appeared.
It reported `waiting_resources`, job `star-g3-base-640`, error:
`insufficient measured GPU budget: -16MiB`.

STaR metadata changed: current response file14 records/11 forced ends, mean
1714.07 thinking tokens. The old30-record1024-budget file was archived as
`responses-budget1024.jsonl`. This is generation progress, not admitted trace
count or quality evidence. FANToM's expected report.json was absent, and no
final score is certified. No failed queue files were listed at the snapshot.
No resource telemetry was collected by Codex; the GPU message is the worker's
own diagnostic. A live process is not proof of useful GPU progress.

## Acceptance fixes verified and remaining counterexamples

`star-acceptance-recheck-20261018.json` binds the updated source SHA256
`5a78f5194e9dac3d43ee98c287c69f85a1818342a44fa56b82c314854aa8b7c5`.

- The earlier English contradiction cases now reject. Missing token count,
  negative count and NaN also reject. The supplied-solution phrase now rejects;
  ordinary Spanish `despistada` is allowed rather than blocked by `pista`.
- The early non-delimiter completion fixture now returns `forced_end=true` and
  rejects. This resolves the exact earlier code-path reproduction; it does not
  certify all native server stop reasons on hardware.
- Remaining: gold `no` accepts `No. En realidad sí.`; gold `less full` accepts
  `Less full is incorrect; more full is the answer.`. The opposite-word check
  only partly covers yes/no labels. Missing `forced_end` still admits a row
  with a positive integer token count. Require explicit completion evidence and
  unambiguous per-task answer syntax; keep raw output for human review.

There are15 acceptance probes and a separate completion fixture. Three desired
rejections still accept (two new language/comparative cases and the old missing
flag case). The expanded set differs from the first audit: its counts are not
a model-quality metric or a live contamination estimate. No labels were changed.
Full-sequence fit, immutable resume identity and self/rationalized cohort
accounting from the earlier review still need evidence beyond lexical parsing.

## Resource lifecycle: confirmed source risks, live cause unconfirmed

`star_stage` enters NativeChat before per-item preflight (`worker.py:429–438`);
benchmark generation has the same ordering. Preflight computes
`min(7168 - whole_device_used, whole_device_free - 768)` with current defaults.
The loaded owned server contributes to whole-device usage. This is an absolute
safety bound, **not literal double counting**. A post-load rejection exits the
context, releases that owned server, requeues and sleeps120 seconds. A context
that deterministically exceeds the bound can repeatedly reload and reject.

The `-16` diagnostic alone cannot identify the cause: under these defaults,
used7184 or free752 can be the binding measurement. Owned server allocation,
external allocations, retained context or a combination remain possible.
RAM is checked after this GPU condition; it cannot directly produce this message
but a simultaneous RAM shortage could be hidden. Do not bypass the safety floor
or subtract owned memory from the absolute used-memory bound to make it pass.

**Separate cleanup defect:** `NativeChat.__enter__` starts its child but raises
on health timeout without terminating it or closing its log. Python does not
call `__exit__` when `__enter__` fails; see the [primary language reference](https://docs.python.org/3/reference/compound_stmts.html#the-with-statement).
`native-lifecycle-20261018.json` reproduces this with an AST-extracted class and
a fake child/clock: healthy exit and post-entry resource exception both clean
up; health timeout leaves zero terminate calls and an open log. No real child
is started by this test, and no live process leak has been measured. This
defect could compound retries, but is not established as the cause of `-16`.

Suggested verified slice for Claude: clean up only owned resources on failed
entry, add lifecycle regression tests, and record full resource snapshots plus
phase/effective context before launch and after health. Distinguish persistent
self-allocation failures from transient external pressure. Preserve checkpoints,
saved responses, owned-child identity and existing hard resource limits.

The observed development config now says sequence_length4096, row_chunk2048;
NativeChat context defaults to32768 independently. Actual job settings and the
revision loaded byPID14364 were not proven by source inspection. Right-size
generation context only after measuring the full prompt + reasoning + answer
bound; a longer thinking cap is not automatic evidence of full training fit.
Earlier1024-specific advice must be read with this new config in mind. Old
strict-parity receipts do not certify changed native sources, caches or4096
training. Keep comparable compute/exposure and frozen evaluation protocols.

Resource reviewer source hashes:

| Source | SHA256 |
| --- | --- |
| resources.py | a92d2bdef4fe82359754a9f8c0d7f0bac4e02fcc29a91555975c19080243d531 |
| contracts.py | 126bdfe6908877fc51f7abe380e8ec3eae94b4be53684b2124b5d30931e7aebf |
| native_chat.py | 79dee516e1bdf5bb3048af77587c89de022c42d0e8cf8b0b0aab675f4ab05446 |
| worker.py | 9a5862d5100373d0917378a223e8210f3c31878d6533de38265df0d9c81ca280 |
| rtx3070-development.toml | ea13e42e39ca75b47937439ae766fba5a4bb0769f3b5fd090004b96b96285671 |

Reproduce CPU receipts with `audit_star_acceptance.py` and
`audit_native_lifecycle.py`, passing the corresponding --source and a new --out
path. Both use only the standard library and AST extraction. They test software
contracts, not hardware, benchmark accuracy, thermal recovery or deployment.
All20 candidate families remain pending-human. Own semantic fine-tuning remains
unrun; no uncoordinated GPU attempt, merge or adapter promotion is requested.
