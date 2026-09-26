# TFOG / Archie JARVIS Orb Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans. Implement each task in order and record progress in the task ledger.

**Goal:** Integrate the TFOG Orb with existing ArchieOS speech, cognition, Stage, follow-up and Accessibility flows, render it in the native observatory, and prepare a WSL build plus PR to `main`.

**Architecture:** Producers publish bounded, local-only signals into `archie-displayd`; its read-only Orb model produces `/api/orb` frames; `ArchieBridge` and Qt Quick 3D render those frames inside the existing application. Policy/Stage retain execution authority.

**Tech Stack:** Rust workspace, Tokio, Unix datagrams, JSON/Serde, C++20, Qt 6 Quick 3D/QML, systemd, Arch Linux/WSL2.

**Spec:** `docs/superpowers/specs/2026-09-23-tfog-archie-orb-design.md`

## Global Constraints

- Orb signal maximum payload: 16 KiB; entity token dimension: 256; graph neighbor budget: 24; focus record budget: 128.
- Orb entity budget: 12,000; frame API is read-only; no Orb-to-Stage command path.
- No audio, transcript, speech text, accessible label, credential or file content in signals.
- Do not reimplement existing speech, hands, Stage, follow-up or Accessibility components.
- Preserve existing WSL `/archie`, `/vault` and user data.
- PR target is current `main`; do not merge it.
- Do not claim that a model/runtime is installed or active unless the WSL installation confirms it.

## Review Focus

- Absent/stale producers: decay transient visual state and retain only current persistent entities.
- Stage failure before receipt: emit an ERROR or allow a bounded expiry; never show VERIFIED without existing follow-up evidence.
- Multiple operations on one surface: reconcile by app/surface and newest observed operation.
- Missing Accessibility relation: do not create a dangling edge or fabricate the parent node.
- Lower-tier hardware or absent GPU: reduce bounded visual work while preserving the same semantic signal mapping.

---

### Task 1: Complete the TFOG model and signal publisher

**Files:**
- Modify `core/crates/archie-orb/src/signal.rs`
- Modify `core/crates/archie-orb/src/model.rs`
- Modify `core/crates/archie-orb/src/transport.rs`
- Modify `core/crates/archie-orb/src/lib.rs`
- Modify `core/crates/archie-orb/Cargo.toml`
- Modify `core/Cargo.toml`
- Modify `core/Cargo.lock`

**Interfaces:**
- Producers call a non-blocking `publish(&Signal)` helper using the configured local socket.
- `Signal` carries bounded entity, Listen, Think, Speak, Focus, Stage and AccessibilitySelection payloads.
- `OrbModel::apply(Signal)` validates and updates model state; `frame(now_ms, dt)` returns a serializable bounded `OrbFrame`.

- [ ] Use the existing 256D signal and model scaffold; fix missing-node focus behavior, dangling relation construction, sensitive placeholder labels, and action status lookup by app/surface.
- [ ] Finish deterministic sparse graph projection, continuity stabilization, adaptive field resolution, radial/harmonic surface values, morphotype selection and collision solving within the approved entity/neighbor budgets.
- [ ] Add producer convenience publication that is non-blocking, optional when the socket is absent, and bounded to 16 KiB.
- [ ] Run `cargo check -p archie-orb --locked` and resolve compiler errors/warnings introduced by this task.

### Task 2: Aggregate real ArchieOS state and action signals

**Files:**
- Modify `core/crates/archie-displayd/src/orb.rs`
- Modify `core/crates/archie-displayd/src/lib.rs`
- Modify `core/crates/archie-displayd/src/main.rs`
- Modify `core/crates/archie-stage/Cargo.toml`
- Modify `core/crates/archie-stage/src/operate.rs`
- Modify `core/crates/archie-state/Cargo.toml`
- Modify `core/crates/archie-state/src/cognitive.rs`
- Modify `core/crates/archie-sight/Cargo.toml`
- Modify `core/crates/archie-sight/src/lib.rs`
- Modify `distro/display/archie-displayd.service`

**Interfaces:**
- `/api/orb` serves the current serialized `OrbFrame`.
- `archie-displayd` turns authoritative module/model snapshots and existing kernel facts into live entity signals.
- Stage emits app/surface/pid plus status only at existing Policy/receipt boundaries; Sight emits resolved Accessibility selection and existing follow-up result.
- Successful persisted focus records publish node IDs and strengths only after the state write succeeds.

- [ ] Complete bounded snapshot-to-entity projection and stale-state behavior.
- [ ] Verify receiver socket ownership/group/mode match the existing `archie-displayd` sandbox and producer accounts.
- [ ] Emit PREDICTED/ACTIVE/ERROR from the existing Stage operation boundary, preserving admission, seal and receipt behavior.
- [ ] Emit VERIFIED/ERROR only from the existing repaint follow-up; reconcile status by app/surface without inventing results.
- [ ] Emit AccessibilitySelection from the selected, resolved Accessibility control while withholding labels/text.
- [ ] Emit Focus from persisted memory events and bounded working-set features from Cerebra's existing trace/projection.
- [ ] Run `cargo check` for the affected Rust packages with the workspace lockfile.

### Task 3: Connect ASR and Magpie TTS signals

**Files:**
- Modify `core/crates/archie-voice/Cargo.toml`
- Modify `core/crates/archie-voice/src/conversation.rs`
- Inspect/modify only existing audio helpers under `core/crates/archie-voice/src/audio.rs` if the chunk envelope is not already exposed.

**Interfaces:**
- Active ASR emits Listen true/false through the existing voice session.
- Magpie playback emits `Speak { turn_id, semantic_features, audio_envelope }` for current PCM chunks, and emits inactive state after the existing playback ends.

- [ ] Publish LISTEN from the actual existing recognition session lifecycle; do not create a second microphone or ASR worker.
- [ ] Build a bounded transient semantic feature vector at the TTS producer without transporting/storing raw speech text.
- [ ] Derive amplitude from existing PCM chunks, normalize to `[0,1]`, and throttle to the existing audio cadence.
- [ ] Ensure signal failure never blocks or alters recognition, synthesis or playback.
- [ ] Run `cargo check -p archie-voice --locked`.

### Task 4: Render Orb frames in the existing native observatory

**Files:**
- Modify `display/native/src/ArchieBridge.h`
- Modify `display/native/src/ArchieBridge.cpp`
- Modify `display/native/src/main.cpp` only if display targeting/fullscreen flags are needed
- Modify `display/native/qml/Main.qml`
- Modify `display/native/CMakeLists.txt` if a focused QML component is added
- Modify `display/visual/*` only when the existing visual model offers the correct stable abstraction

**Interfaces:**
- ArchieBridge polls the bounded `/api/orb` JSON projection and exposes revisioned orb data to QML.
- The native renderer consumes frame node, edge, field, morphotype, action and voice values without command methods.

- [ ] Add a separate timer/revision for the bounded Orb projection; do not increase the daemon's control-plane polling cadence.
- [ ] Draw the Orb's bounded native surface, semantic nodes/edges, action status and voice pulse within the existing observatory UI.
- [ ] Apply hardware-tier render budgets; degrade frame/mesh quality when GPU resources are unavailable.
- [ ] Add command-line support for deterministic full-screen/display selection only if required for WSLg, while retaining the existing `--render-check` path.
- [ ] Configure a WSLg build and run the native compiler/link step; capture a real render snapshot.

### Task 5: Package, update WSL, snapshot and publish the PR

**Files:**
- Modify `scripts/wsl/build-rootfs.sh`
- Modify `scripts/wsl/package-wsl.sh` only for required display artifacts
- Modify `distro/wsl/rootfs/usr/lib/systemd/system/*` for required WSL units/target wiring
- Modify `distro/display/archie-display.service` or add a WSL-specific override if WSLg must avoid EGLFS/DRM
- Modify `distro/wsl/rootfs/usr/lib/systemd/system/archie-speech.service` only if the existing voice unit needs enablement/preservation
- Add progress artifacts under the session snapshot output directory, not into the runtime or source tree unless requested

**Interfaces:**
- WSL uses the built branch binaries and existing model assets/installer locations; installation is non-destructive and preserves `/archie` and `/vault`.
- The PR source branch is `codex/tfog-archie-orb`, based on the fetched `origin/main`; destination is `main`.

- [ ] Build the release daemon, native Qt renderer and required WSL runtime/package files from the current branch.
- [ ] Inspect current model/runtime installation. Install an already-present Nemotron model if found; otherwise clearly report which artifact is missing before any large model download.
- [ ] Update the existing ArchieOS WSL system in place without unregistering or replacing user state.
- [ ] Start ArchieOS in WSLg on display 2 in full-screen mode; confirm actual daemon/model status.
- [ ] Capture progress and final screenshots from the real running WSL renderer.
- [ ] Run final `cargo check` and native build; inspect `git diff`, commit the feature, push the approved source branch, and open a PR to current `main`.
- [ ] Report the PR URL and ready-to-merge status without merging it.
