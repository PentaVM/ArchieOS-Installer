# Source launch — 2026-10-19 04:58 UTC

Owner requested the current source build with Bonsai and the new design, before
continuing PR41. Origin verified as PentaVM/ArchieOS-Nvidia. Checkout and PR41 head:
`5c86b46fa4b8f792ab3095aded4eb78c90c1bdf1`; no release installation or merge.

## Running development session

- WSL `ArchieOS-Operator-RTX3070-Test`, user archie. Private state:
  `/home/archie/.local/state/archie-pr41-dev`, separate from the installed node.
- Rust development build completed with `--locked`, `CARGO_BUILD_JOBS=2`, packages
  archie-cli, archied, archie-displayd, archie-shell-bridge, archie-voice and native
  speech enabled. Binaries in `/tmp/archie-xavi-cargo/debug`; source native Qt host
  in `/tmp/archie-xavi-native/archie-display`.
- Real private PostgreSQL on loopback55441, TimescaleDB2.29.2 and vector0.8.6.
  Core healthy; local Bonsai manifest registered. Initial missing extensions and
  missing manifest cost were corrected only in this private session.
- Native visible WSLg shell, read-only shell bridge and displayd are running.
  `polar-prisma` received a successful control ACK. Runtime catalog lists all eight
  `polar-*` profiles; `/api/orb/lumen` returns actual Core/OS observations (no fixture).
  llvmpipe renders on CPU to avoid competing with the training GPU.
- Exact Bonsai2 9B native PTQ1_0 base, 1,997,486,976-byte existing GGUF, served by
  local llama.cpp b10777 on loopback8081, CPU only, two threads, context4096,
  parallel1, GPU layers0 and CUDA devices hidden. No Wanda adapter is attached.
- Actual non-streaming inference returned `¡Hola!`, natural stop, 19 prompt tokens
  and four generated tokens in10.41s. Health and model registry were also checked.
- Native NeMo CPU ASR and bilingual CPU Piper started against WSLg microphone and
  speaker. Runtime reports listening for `Archie`; wake-word boosting is unavailable
  because this ASR GGUF lacks an embedded SentencePiece tokenizer.

## Limits and pending work

The full source voice prompt exceeded the existing90s provider timeout during CPU
prefill. A running microphone/ASR does not establish a successful spoken conversation;
the short inference check above does not establish usable conversational latency.
Stage/IRIS tool execution is not connected in this session. No release image, installed
ISO, VBox or NVIDIA renderer verification is claimed. Existing fidelity limitations
in `source-port.md`, including the separate selective-bloom pass, remain open.

Training worker/watchdog/holds were not changed or stopped; no additional CUDA job
was launched. Process identities and logs are persisted in the private session, not
committed credentials. Launch helpers remain outside Git under the local handoff
directory. This receipt does not mark PR41 ready or its remaining gates complete.
