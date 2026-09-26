# TFOG / Archie JARVIS Orb — Approved Design

## Goal

Add the TFOG semantic Orb to ArchieOS by observing its current ASR, Cerebra, Magpie,
Stage, follow-up and Accessibility flows. The Orb is a visualization of live system
activity; it never grants capabilities, proposes authority or executes Stage actions.

## Approved boundaries

- Existing speech recognition, speech synthesis, hands/Stage, action verification and
  Accessibility implementations remain the source of their signals. They are not
  reimplemented.
- `archie-displayd` is the local aggregator. Producers send bounded, one-way Unix
  datagrams; the renderer reads a read-only `/api/orb` frame. No return command path exists.
- Policy and Stage remain the only authority for executing actions. The Orb observes
  PREDICTED, ACTIVE, VERIFIED and ERROR state.
- Signal payloads do not contain audio, transcripts, spoken text, accessible labels,
  credentials or file contents. TTS semantics are reduced to a transient bounded feature
  vector at the producer; the audio envelope carries only normalized amplitude.
- Entity IDs and minimal relationships are used to relate App, Window, Process, Control,
  File, Device and runtime nodes. Accessibility links are omitted when the relationship
  cannot be resolved.
- The existing Qt Quick 3D observatory owns rendering and window lifecycle. The Orb is an
  additional native scene, not a second application or web view.
- Hardware profiles change bounded node budget, field sampling, animation cadence and
  quality while retaining the same normalized TFOG algorithm and meaning.

## Runtime flow

1. ASR emits LISTEN activity only while its existing listening/recognition flow is active.
2. Cerebra/memory emits ranked working-set nodes; `archie-state` focus records supply
   longer-lived focus strengths.
3. Stage/hands emits the affected app, surface and process after Policy admission and
   before dispatch. The resulting receipt marks ACTIVE or ERROR; the existing repaint
   follow-up advances the matching surface to VERIFIED or ERROR.
4. Magpie emits speech turn identity, an ephemeral semantic vector and bounded audio
   amplitude while playback chunks are active. Speech semantics affect only relevant
   nodes; amplitude drives pulse strength.
5. Accessibility emits selected app/window/control identity and a resolvable relation,
   with no control label or text content.
6. `archie-displayd` applies TTLs, validation and aggregation, then serves a serialized
   Orb frame. Socket and route failure cannot delay or alter producers or execution.

## TFOG frame and rendering

The core model keeps bounded 256-dimensional entity signatures, sparse k-nearest
semantic relationships, a normalized graph spectral embedding with continuity-preserving
orientation, focus-weighted radial geometry, spherical harmonic surface coefficients
(degree up to 8), morphotype selection and spatial-hash collision projection. State and
voice envelopes use bounded, smoothed dynamics; stale transient signals decay and expire.
Frames contain positions, visible relationships, local focus/action/voice strengths,
state, morphotype and resource profile. The native bridge parses these frames into Qt
scene data and renders them within the existing observatory.

## Device and resource behavior

The same graph and normalized geometry apply across devices. The profile derives from
available CPU/GPU and current display budget; it limits entity count, solver work, field
mesh resolution and frame cadence. Degraded GPU/display support lowers visual quality
without changing signal meaning or the authority boundary.

## WSL delivery

Build from the current `origin/main` plus this feature branch. Update the existing WSL
ArchieOS installation without unregistering the distro or deleting user state. Start the
native observatory in WSLg full-screen on display 2. Report whether the existing Nemotron
speech runtime/weights and deterministic Stage click/control path are actually available;
do not mark an unavailable model or service as running. Provide real rendered snapshots
of the current WSL OS and the Orb. Open a PR targeting `main`; leave merge authority with
the user and report when the PR is ready to merge.

## Review conditions

- No producer can invoke Stage or bypass Policy.
- No raw transcript, audio, accessible label or file content enters Orb transport.
- Malformed, oversized, non-finite or stale signals cannot crash the daemon or poison a
  frame.
- Missing producers/socket/daemon produce an empty or decaying frame, not invented live
  activity.
- UI refresh and Orb rendering stay bounded on lower-tier hardware.
- WSL update preserves `/archie`, `/vault` and existing user files.
- PR base includes the latest `origin/main`, including the already-merged voice work.
