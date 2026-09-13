# Local Nemotron speech

ArchieOS uses `nvidia/nemotron-speech-streaming-en-0.6b` only for ASR. It is not a
planner, a text generation model or Cerebra. The default alpha runtime is
NeMo-Speech.cpp at commit `a5b6953c4a579a2bbd1c0913ad8a85c2a4d99953` with the
Q8_0 GGUF. The expected model SHA-256 is
`d9a01898d2a611c8764e23a1c2f45e70bbd5a425dc4de93692ac951dd603812d`.

Install inside ArchieOS:

```bash
sudo /usr/lib/archie/install-nemotron-speech
```

The installer builds the pinned NVIDIA source revision, downloads the named model over
HTTPS, verifies its checksum before rename, and writes only revision/checksum metadata.
No model or proprietary CLI is committed to the repository.

`LocalSpeechRuntime` launches `nemo-speech --json transcribe AUDIO --model MODEL
--stream` without a shell. Live output uses NVIDIA's `[live partial @ ...]` and
`[live final @ ...]` records; diagnostics are ignored. Partial transcripts are runtime
events and are never admitted to memory automatically. Final text still needs Cerebra's
learning admission decision.

Audio samples live in a non-serializable, 30-second bounded `EphemeralAudio` buffer.
NaN and infinity become silence, finite values are clamped, and the buffer is overwritten
on drop. The speech adapter contains no network client. The existing independent acoustic
floor estimator receives its own ephemeral features before transcript finalization.

The current alpha is English-only. No accuracy claim is made. Latency must be measured
on the target GPU/CPU using an ephemeral user-authorized sample; no raw audio fixture is
kept in Git.

