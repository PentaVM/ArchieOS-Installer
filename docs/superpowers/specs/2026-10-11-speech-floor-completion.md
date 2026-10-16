# Speech floor completion and branch integration

The owner requests completion of existing speech-to-speech and integration of
pending branches, exclusively in ArchieOS-Nvidia. GUI and WSL generation belong
to Claude/Xavi. This is completion of existing flows, not a replacement assistant.

Keep the native ASR, Piper/Magpie/RVC, SpeechSession, FloorPolicy and tool authority.
Expose the existing streaming recognizer behind a small audio boundary so the
actual conversation loop can be exercised without linking NVIDIA's SDK. SDK
handles and decoding stay in native.rs.

Interruption must use current ASR evidence, leave acknowledgements alone and
yield on a substantive overlapping utterance. Unknown sustained speech has a
bounded 320 ms recognition grace period. A final event is not invented silence:
the loop uses measured quiet and retains unfinished clauses across ASR endpoints.
Never discard a finalized correction while old audio drains. Cancelled generations
must not mark new response segments as heard. A completed audio segment is recorded
after playback drains, not merely after synthesis. Shutdown invalidates queued audio.

Complete the pre-existing bilingual Piper work without replacing newer RVC or
live-memory integrations. Keep sample rates consistent and report missing voice
models explicitly. No model downloads or dependency installation for validation.

Integrate Sentry's committed branch; preserve GUI/WSL/render work in other
worktrees. A patch-equivalent ancestor is not merged again solely to change history.
Report tests, omissions and real-device/model limits, with a reproducible replay.
