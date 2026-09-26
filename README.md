# ArchieOS NVIDIA Edition

ArchieOS is a personal AI that lives in an operating system. You talk to Archie; it
answers out loud, sees through a camera, opens and uses applications, and shows what it
is doing as a living graph of the machine. Identity, memory, policy and authority live in
ArchieOS; language and vision models are replaceable resources it calls.

One rule holds everywhere: **models propose, Cerebra decides, Policy authorises, the
Stage executes.** A model never touches the machine directly. Every action is a sealed,
leased Stage command that the policy can refuse, and every result comes back as a receipt.

```text
 microphone ─► Nemotron 3.5 ASR ─► floor ─► Nemotron (Nebius) ─► voice (Piper / Magpie)
                                               │ proposes
 camera ─► IRIS ─► scene + memory ─────────────┤
                                               ▼
                                 Cerebra ─► Policy ─► Stage ─► applications
                                               │
                         TFOG signals ─► archie-displayd ─► Observatory (World · Forge · Orb)
```

This repository is the NVIDIA / Nebius competition edition: Rust for the control plane,
C++ for the native and visual subsystems. Nothing in ArchieOS itself is Python.

## What works today

**Talking to Archie**
- `archie-voice converse`: speech to speech on the machine. Nemotron 3.5 streaming ASR
  (Spanish and 39 other language-locales) through NVIDIA NeMo-Speech.cpp, a floor that
  decides whether Archie was addressed ("Archie", or anything while it is listening),
  Nemotron on Nebius for the answer, and a voice: Piper on a CPU, Magpie with a GPU.
  Answers are spoken in short sentences, in the language the person used.
- `archie-voice live`: the same conversation reached from a browser page (microphone,
  speaker and camera over a loopback WebSocket), with **IRIS as Archie's eyes**. Archie is
  told only what IRIS reads, how old it is, and what was there before and left. It can
  look closer at an object, find out what it is with a vision model on Nebius (Kimi K3,
  then MiniCPM-V 4.5, on the conversation's own key) and read about it, keep that until
  asked, and it never identifies people: faces are blacked out of every crop.
  A gentle voice style is applied after synthesis. Tested end to end, speech to speech.
- `display/stage/run-archie.sh`: one command for the stage and the voice together.

**Acting on the machine**
- **Archie Stage** (`display/stage`): a Wayland compositor where Archie works. Commands are
  HMAC-sealed and leased; the policy (`core/crates/archie-stage/policy/apps.toml`) names
  which applications may be launched and what may be done in each. Clicks, typing, keys,
  held keys (released automatically after 5 s or when the operator takes over),
  scrolling, drags and strokes; the person's mouse always wins.
- **Hands** (`archie-voice`, `archie-sight`): Nemotron proposes a tool call; it is checked
  strictly and carried out through the Stage gate, grounded by what applications declare
  over accessibility first, a local reflex model second, and a vision model last.
- **Video editing** (`archie-cut`, `archie-mcp-server --video-media`): timelines of clips,
  titles, music and fades rendered by MLT (Kdenlive's engine), without a window.
- **IDE agents** (`archie-mcp-server`): Archie's tools over MCP.

**Seeing**
- **IRIS** (trained in [`PentaVM/archie-iris`](https://github.com/PentaVM/archie-iris)):
  open-vocabulary detection. An image and phrases in; boxes, points and a score out, plus
  an identity embedding per detection. Beta 1.1 (754 M parameters, V-JEPA 2.1 + SigLIP2 +
  sparse MoE) runs at 159–217 ms per image on an RTX 3090. Beta 2 (512 px, 32 experts,
  GUI, sport and text data) is training.
- `archie-iris-bridge`: turns an IRIS proposal into a Stage request that Cerebra and the
  policy still have to admit.

**Showing what happens: the Observatory** (`display/native`, Qt Quick 3D)
- **World**: the Earth, lit by the real sun, with real observations.
- **Forge**: the machine's applications as a desktop — Archie's own, everything installed,
  and the well-known tools that are missing.
- **Orb**: the live TFOG graph of the system (processes, services, devices, modules,
  models). What the system is using lights up — a Stage action, Cortex's focus, the voice
  — and the camera moves when something happens. See
  [`docs/TFOG-ORB-RENDERING.md`](docs/TFOG-ORB-RENDERING.md).

**Underneath**
- Rust authority, policy, scheduler, model router, memory and PostgreSQL/TimescaleDB state;
  Cerebra (clarification, bounded analytical synthesis); the Vault; Cortex and Sentry in
  C++; OSM building ingestion; ArchISO and WSL2 packaging.
- Model access is provider-neutral: NVIDIA NIM, Nebius Token Factory, or a local
  OpenAI-compatible server. Credentials are read from the environment at request time
  and never written to the repository or the image.

## Run it

Build the Rust workspace (Rust 1.88+):

```bash
cd core
cargo build --release -p archie-voice --features speech   # converse
cargo build --release -p archie-voice --features live     # live, with the browser page
cargo test --workspace --locked
```

`speech` and `live` need NeMo-Speech.cpp installed (`NEMO_SPEECH_PREFIX`) and PulseAudio;
`scripts/models/install-nemotron-speech.sh` fetches the pinned ASR model.

Talk to Archie, with the Stage:

```bash
NEBIUS_TOKEN_FACTORY_KEY=... display/stage/run-archie.sh public
```

The live session with eyes (IRIS served by `archie-iris` on the GPU host):

```bash
archie-voice live --grade public --asr-model <nemotron-3.5-asr.gguf> \
  --voice-engine magpie --tts-model <magpie.gguf> --codec-model <nanocodec.gguf> \
  --tts-tokenizer <dir> --iris http://127.0.0.1:8765
```

The page listens on loopback (`--port`, default 8770); reach it through an SSH tunnel.
`ARCHIE_LIVE_VLM` (another vision model list) and `TAVILY_API_KEY` (web reading) are optional.

The Observatory (Qt 6.6+ with Quick 3D) and its data daemon:

```bash
cmake -S display/native -B build/display -G Ninja && cmake --build build/display
core/target/release/archie-displayd &
build/display/archie-display --scene orb      # or world, forge
```

`scripts/check.sh` runs the checks worth running before a push. The native libraries have
their own tests (`-DARCHIE_VISUAL_BUILD_TESTS=ON`, `-DARCHIE_CORTEX_BUILD_TESTS=ON`,
`-DSENTRY_BUILD_TESTS=ON`).

## ArchieOS Workstation for WSL2

An Arch Linux WSL2 distribution with systemd, a non-root `archie` session, a sealed Vault,
local Nemotron ASR, provider adapters and FreeCAD. Build with
`sudo ./scripts/wsl/build-rootfs.sh ./dist` and install with
`wsl --install --from-file .\dist\ArchieOS.wsl`. See [WSL build](docs/WSL-BUILD.md),
[first run](docs/FIRST-RUN.md) and [security boundary](docs/SECURITY-BOUNDARY.md).

## Status

- **Working and tested:** speech to speech (converse and live), the Stage and its policy,
  hands through accessibility and vision, video rendering, the Observatory's three
  scenes, IRIS Beta 1.1 detection behind the live session, the control plane and packaging.
- **In progress:** IRIS Beta 2 training; Cortex driving the Observatory's view by command;
  IRIS proposing actions beyond clicks.
- **Not yet:** an IRIS inference runtime inside ArchieOS (it runs as the `archie-iris`
  service today); opening applications from Forge in the ArchISO image, where the
  Observatory has no desktop and opening goes through the Stage; CI (GitHub Actions is
  paused for billing — checks run locally).

See [ARCHITECTURE.md](ARCHITECTURE.md), [HACKATHON.md](HACKATHON.md),
[`docs/STAGE.md`](docs/STAGE.md), [`docs/SPEECH-NEMOTRON.md`](docs/SPEECH-NEMOTRON.md) and
[`docs/IRIS.md`](docs/IRIS.md).

## License

Apache License 2.0 — see [LICENSE](LICENSE) and [NOTICE](NOTICE). The Apache licence
rather than MIT for the patent grant: this repository contains device control, physical
actuation and inference-routing work.

Upstream data providers keep their own terms. `providers/` records the attribution each
one requires, and the registry refuses to display records from a provider whose
attribution is missing.
