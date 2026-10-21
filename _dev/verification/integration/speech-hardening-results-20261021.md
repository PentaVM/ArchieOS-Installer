# Speech/floor hardening — 2026-10-21

Repository: PentaVM/ArchieOS-Nvidia. Source baseline main `bca8473`; isolated branch `codex/speech-floor-hardening`. Owner authorized autonomous voice/floor improvement for the next manual trial, preserving Claude's Wanda/LNCM, GUI/WSL and active processes. No training or GPU inference was started.

## Resulting behavior

- VAD consumes measured sample duration rather than assuming every packet is 20ms. The existing browser route already splits packets into 20ms frames; variable chunks from other stream callers now behave consistently too. Empty audio is not silence. Invalid/oversized streamed PCM fails before reaching native ASR.
- Both interactive CLI routes dispatch cancellable bounded futures instead of blocking the hearing loop on tools. Pending work belongs to one admitted turn; a named replacement cancels it, keeps the name evidence from either recognizer, and discards queued proposals. The old synchronous embedding API is retained explicitly.
- Fixed a pre-existing tool identity mismatch: the real adapter emits `turn#ordinal`; the session previously compared the whole string against `turn`, silently dropping its proposals. Only an exact turn or its strictly numeric ordinal suffix is accepted. Policy still decides each action.
- At most two speech segments are released toward playback. Model polling stops at the first event and after at most 32 invisible updates; segmentation retains a large combined answer and releases one piece at a time. Text deltas over16KiB or cumulative step text over64KiB abort explicitly.
- Removed an unscoped `Idle` playback receipt which could arrive after replacement speech was queued and erase its count. Every queued `Say` already returns a turn-scoped receipt, including when cancelled. Failed model turns terminate, abort owned actions and clear Orbe's thinking trace.
- Added turn-correlated diagnostic phases and a separate CPU trial launcher. Original desktop/GUI launchers and installed binaries remain owned by their existing developers.

## Evidence ledger

All cargo work used one build job and `CUDA_VISIBLE_DEVICES=` in the existing operator WSL distro. The full default workspace suite passed **2427 tests,108 ignored**, before the final additional regressions. Workspace `--all-targets` Clippy with `-D warnings` also passed at that checkpoint. Final scoped results and native smoke receipts are recorded below after execution.

Meaningful RED cases were observed before fixes: missing asynchronous scheduling API; a slow tool preventing a correction; real `turn#0` proposals being dropped; late unscoped Hush receipt; name lost by the second recognizer;1000 prequeued updates parsed in one poll;1000 sentences in a single delta segmented in one poll; invalid streamed PCM accepted. Each relevant regression was then run green. Existing floor replays cover unfinished clauses, acknowledgements, correction overlap and stale playback receipts.

Fresh independent review found and drove fixes for Idle/Hush, recognizer disagreement, adapter/segmenter boundedness and Orbe failure closure. Reviewer re-read repairs and the isolated smoke/launch scripts and reported no remaining blockers. This was static review, without builds or model loads by the reviewer.

Strict feature lint also exposed baseline warnings: RVC numeric literal grouping, a native live-stream `next` API which is deliberately not an exhausted iterator, an unchecked fixture read count, and a complex fixture item type. These received narrow fixes/documentation; no blanket lint suppression or GUI behavior change was introduced.

Full workspace checkpoint after production repairs: **2432 passed,108 ignored**, strict workspace all-target Clippy passed. Further end-to-end tests then proved multiple async tool results reach follow-up exactly once/in order, backend failure finishes the real loop without waiting for shutdown, and explicit solo mode supersedes a pending tool even when ASR omits the name. The solo-mode guard was tested RED before changing it; normal mode still requires address evidence. Final scoped `speech,live,rvc` suite including those tests: **213 passed,1 ignored**; strict Clippy for both speech/voice all targets and a linked CLI build passed. Counts overlap and must not be added. `cargo fmt` and diff whitespace checks passed. The ignored RVC test requires real opt-in models/runtime inference.

Final native smoke passed on CPU affinity0,1 with lower priority: Nemotron ASR loaded through NeMo-Speech.cpp `nemo-speech-asr0.1.0`, actual Piper input/output, one scripted loopback SSE request,64768 output frames at22050Hz. Exact final binary SHA256: `c229c655c4daae2f87d5136a4f718595f1528fdefb52fa10b02ae98c15467c98`. Receipt and synthetic WAVs are under the local handoff `voice-floor-trial/native-smoke`. Input used `--always`, not a successful wake-name assertion. The ASR reported no embedded SentencePiece tokenizer for RNNT phrase boosting and transcribed the synthetic «Archie» as «Aquí». This is one observed fixture, not an accuracy rate. The trial launcher offers explicit `-Always` for a solo test; common word «aquí» was not promoted to a wake alias. No microphone/speaker or Bonsai inference occurred in this smoke.

## Verification limits

Controlled ASR/model/audio tests establish state transitions, not model accuracy or human-perceived latency. The native smoke uses real CPU ASR and Piper with a scripted loopback reasoning server and synthetic input; it must not be described as Bonsai quality, microphone, speaker or physical AEC verification. Optional RVC model inference stays opt-in. Existing optional second-pass ASR/native synthesis/PulseAudio blocking boundaries remain; details and operator cases are in `docs/VOICE-FLOOR-TRIAL.md`.

This branch does not rebuild the earlier WSL release image and does not adopt or modify Claude's Wanda/LNCM work.
