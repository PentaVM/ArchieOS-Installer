# Floor/voice overnight hardening

Owner brief: improve the existing conversation for tomorrow's manual trial, autonomously, using CPU for verification and leaving GPU/training and Wanda/LNCM to Claude. Preserve authority, domains and GUI/WSL ownership. Source baseline main bca8473; branch codex/speech-floor-hardening in the existing isolated review worktree. The owner's explicit autonomous implementation request supersedes skill review/approval handoffs; execute inline and get a fresh branch review at the end.

Design: retain SpeechSession, the existing models and the permission handlers. Correct measured audio timing first. Introduce cancellable asynchronous tool dispatch in the real loop without requiring a new cognition service. Add bounded queues/segment release so a fast model cannot build an unbounded voice backlog. Keep old synchronous callers working. Expose precise turn/phase events for diagnostics and an operator smoke guide. Do not claim learned acoustic calibration, AEC, audible latency or a rebuilt WSL from fixtures.

Options considered: replacing the floor with a new trained model would require data/hardware evidence; forcing Cerebra's different floor schema into the current loop would alter authority and social semantics. Both are excluded from this hardening slice. Canonical integration remains a separately reviewable follow-up; the current loop and its safety boundaries can improve now.

## Execution and proof

- [x] Baseline speech/voice tests, live feature, CPU only.
- [x] Task 1: sample-duration VAD and endpoint timing. RED: equivalent 20/100/160ms audio produces identical silence; invalid levels fail conservatively. GREEN: use actual frame duration in production; early final/unfinished clauses/acknowledgements still pass. Invalid/oversized streamed audio is rejected before ASR.
- [x] Task 2: asynchronous tool results with turn identity. RED: a pending tool cannot prevent the real loop hearing a correction. GREEN: both interactive CLI handlers use runtime futures, old synchronous callers remain supported. Tests cover superseded turns, cancelled pending work, model failure, shutdown and multiple async results reaching follow-up once in order.
- [x] Task 3: bounded speech backlog and structured timing. RED: prequeued updates and a single combined1000-sentence delta both overran poll. GREEN: incremental segmentation, at most32 updates/one segment per poll and two playback segments. Added diagnostic phases; no browser playback ACK claim.
- [x] Task 4a: regression suites, fmt, strict Clippy, feature build and fresh independent review with repairs. CPU builds one job, CUDA_VISIBLE_DEVICES empty. Controlled tests load no models; a separate, bounded two-core smoke uses existing real CPU ASR/Piper with scripted reasoning. No training/GPU jobs.
- [ ] Task 4b: publish isolated PR and prepare the operator trial entry point without overwriting a running installation or Claude's WSL files.

Review focus: stale receipts; empty/oversized stream chunks; failed/disconnected sinks; tool results arriving after interruption; playback backlog during model completion. Each must be covered by the relevant task or explicitly recorded as a limitation.

## Progress ledger

Implementation and review complete. Full workspace checkpoint:2432 passed/108 ignored, strict all-target Clippy passed. Final scoped speech/live/rvc:213 passed/1 ignored and strict all-target Clippy passed. The final extra solo-mode regression was observed RED and then GREEN: after excluding acknowledgements, only explicit --always permits unnamed supersession. Independent rereview found no blockers. Linked feature-enabled CLI and final CPU ASR/Piper smoke passed; binary identity receipt is persisted. Handoff publication pending. See results ledger and operator guide for boundary limits, including native blocking calls, wake-name recognition and optional synchronous second-pass ASR.
