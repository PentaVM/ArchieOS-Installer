# ArchieOS Jarvis workflow orb implementation plan

> Implement inline in the existing `codex/tfog-archie-orb` branch and update PR #1. The user has already authorized this implementation and PR work.

**Goal:** Render a data-driven gold orbital graph and expose real OS/workflow activity without fabricating active components.

**Architecture:** The display daemon observes local read-only runtime facts, `archie-orb` projects a bounded semantic field, and native Quick3D batches luminous field geometry. Existing producers emit small, private workflow signals when work actually occurs.

**Tech stack:** Rust/Tokio, `/proc` and `/sys`, Qt Quick3D/C++, QML, WSLg Vulkan with a field-derived QML light pass.

**Spec:** `docs/superpowers/specs/2026-09-24-jarvis-workflow-orb-design.md`

## Constraints

- Orb remains read-only; Policy/Stage retain execution authority.
- No raw transcripts, frames, arguments, private paths, or social evidence in orbital signals.
- Live and simulated sources must be visibly distinguishable.
- Display daemon should remain under its 128 MiB service limit.
- Keep `main` untouched; update the existing PR branch.

## Work

### 1. Data and field

- [x] Replace the uncentered harmonic surface contribution with bounded normalized angular density.
- [x] Avoid rebuilding the semantic graph for unchanged entity upserts.
- [x] Bound entity counts by source caps and renderer LOD; measure the live low profile response.

### 2. Live local inventory

- [x] Add a read-only collector for process identity/activity, active service membership, and device classes.
- [x] Feed its entities into the existing `OrbService` with separate lifecycle from the `archied` snapshot.
- [x] Confirm source counts and transient process removal in WSL without exposing command lines or private data.

### 3. Native rendering

- [x] Batch field traces and point glints into a small number of meshes; make the envelope faint.
- [x] Fix edge opacity to read the model's `similarity` value.
- [x] Add explicit live entity and workflow activity labels, preserve selection and action colors.
- [x] Capture and visually inspect Core and Orb renders on the current WSL screen.

### 4. Workflow visibility

- [x] Define a bounded workflow observation signal and recent status in the orb frame.
- [x] Instrument existing Sight/Iris/Vision phases at source boundaries without changing control behavior.
- [x] Show Wanda and Cortex as waiting or unavailable until actual source observations exist; add source hooks where a runtime call site exists.

### 5. Delivery

- [x] Build the changed Rust packages and native Qt app; inspect the live endpoint and runtime process state.
- [x] Update the WSL instance and leave the latest Orb scene open full screen on screen 2.
- [x] Commit, push the branch, and report PR checks and merge readiness without merging.
