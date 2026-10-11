# Local Nemotron speech

ArchieOS uses `nvidia/nemotron-3.5-asr-streaming-0.6b` only for ASR — the multilingual
successor of the English-only `nemotron-speech-streaming-en-0.6b`, which cannot hear an
operator who speaks Spanish. It is not a
planner, a text generation model or Cerebra. The default alpha runtime is
NeMo-Speech.cpp at commit `a5b6953c4a579a2bbd1c0913ad8a85c2a4d99953` with the
Q8_0 GGUF at Hugging Face revision `1c8deaecc64b91f034d73e08dd8b64625eb3395d`. The
expected model SHA-256 is
`a5c435f294eea8f88ce68dd27b8c3bfea7f777cb2fbba04fcd30eaa555f429ae` (741,548,352 bytes).

Install inside ArchieOS:

```bash
sudo /usr/lib/archie/install-nemotron-speech
```

The installer builds the pinned NVIDIA source revision, downloads the named model over
HTTPS, verifies its checksum before rename, and writes only revision/checksum metadata.
No model or proprietary CLI is committed to the repository.

`LocalSpeechRuntime` launches `nemo-speech --json transcribe AUDIO --model MODEL
--stream --language auto` (or a fixed locale) without a shell. Live output uses NVIDIA's `[live partial @ ...]` and
`[live final @ ...]` records; diagnostics are ignored. Partial transcripts are runtime
events and are never admitted to memory automatically. Final text still needs Cerebra's
learning admission decision.

Audio samples live in a non-serializable, 30-second bounded `EphemeralAudio` buffer.
NaN and infinity become silence, finite values are clamped, and the buffer is overwritten
on drop. The speech adapter contains no network client. The existing independent acoustic
floor estimator receives its own ephemeral features before transcript finalization.

The configured ASR model is multilingual. This does not establish measured accuracy
for Spanish or any other language. Latency must be measured
on the target GPU/CPU using an ephemeral user-authorized sample; no raw audio fixture is
kept in Git. See `HANDOFF-2026-10-11-SPEECH-FLOOR.md` for the tested floor behavior,
the outstanding native runtime checks and a reproducible benchmark protocol.
