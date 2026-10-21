# Own bounded run: coordination and closeout

Updated2026-10-17. The strict public-text FP32 reference and two mechanical
backward updates are recorded separately in the unchanged `strict-*.json`
receipts. **No own semantic optimizer update has happened.** No adapter was
exported or promoted, and no decision-quality or native-serving claim follows.

The originally authorized semantic launcher correctly failed its old stopped-worker
ownership assumption before creating any phase child. That attempt is preserved in
`own-attempt-blocked.json`; its root contains only configuration/deadline files.

Root subsequently authorized the documented sharing protocol: exclusively create
`/home/archie/wanda/run/hold`, wait for the live worker to pause, use a separate
exclusive Codex lock, then remove only the hold created by this run. The full current
`STEERING.md` and `results/weekly/LOG.md` sections were read. Neither the original
worker lock nor control/source/queue/watchdog files are changed by this diagnostic.

Root found a real bug in the initial diagnostic hold adaptation: outer exception
cleanup could run after the context had removed the hold. The repaired diagnostic
stores its owned child inside `OwnedHold`; on thermal/error/interrupt, process-group
termination and successful wait happen **before** hold removal. If child shutdown
fails, the hold remains. A preexisting or replaced hold is never removed; worker
PID/boot changes or unpaused state reject use. CPU tests first reproduced the
cleanup failure, then all five tests passed, including KeyboardInterrupt, thermal
error, failed shutdown, ownership replacement and changed paused identity.

Fresh2026-10-17 precheck: the live shared worker was PID7864, boot
`062a7c2e-aab4-49f5-85c4-66976ecc27c3`, paused with no active job. No hold preexisted.
The GPU was24°C with1523MiB used, and the CUDA compute-process query still listed
PID7864. The diagnostic requires no other CUDA process/context before each owned
phase, so this precheck prevented hold creation and all child/GPU launches. It does
not prove that the paused process was actively computing. The allocation also adds
roughly800MiB over the background measured in the strict native reference, whose
6640MiB peak already leaves a narrow margin below7168MiB. The temperature/memory
limits were not relaxed to get a run through. See `own-hold-precheck-blocked.json`.

The ready diagnostic is `high_semantic_cabina.py`. It admits only compact candidate
SHA256`a6eff2ddebbe2755c4937a3da9f4aea1737738d49496b9070fc286edeac9202f`, work family
`r2c-cabina-interprete`, all four full variants454/502/455/445 tokens; bono is
explicitly excluded. It would prepare fresh strict-FP32 caches in native ubatch64,
run q/v rank8/alpha16, seed17, lr1e-4, accumulation1 for at most two updates, save
atomic per-step checkpoints, and export only in a separate CPU-only process. The
whole owned experiment remains limited to180s, initial temperature<60°C,
abort>=70°C and total device use<=7168MiB. No automatic retry was made after either
failed precondition. A new attempt requires safe coordination, not changed gates.

Validation on the isolated reviewed code: full trainer CPU suite57 tests,56 passed
and one explicit hardware opt-in skipped; separate diagnostic hold suite5/5 passed;
`git diff --check` clean. No GPU was used for these checks. Strict receipt JSON is
marked `-text` in this directory so Git does not change evidence bytes or their
recorded SHA256 through CRLF normalization.

Shared main advanced from this worktree's base16841cf to652ce23 with Claude's
metrics/worker changes, and root is adding research-only commits separately. This
isolated code must be reviewed as a patch and integrated while preserving those
changes. Nothing here was copied over the live worker's source, committed on its
branch, pushed or deployed by this agent.
