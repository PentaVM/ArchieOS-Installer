# IRIS full-scope baseline

## Audit point

The full-scope audit started from `92226e2` on `origin/main`. The implementation branch
also contained the reviewed workstation series and IRIS Gates B through D. The protected
historical checkout at `C:\Users\Penta\Desktop\Archie` was not used.

The audit covered repository status/remotes/branches, the last 100 graph entries, the last
50 authored commits, and all requested perception, training, dataset, cloud and model terms.
`docs/IRIS-BASELINE.md` records the code-level perception findings. This document adds the
Foundry, deployment and self-improvement baseline required by the full-scope specification.

## Tests before the full-scope continuation

The WSL Arch builder recorded 394 passing relevant Rust tests and one portability failure
before IRIS. Gate D fixed that Windows-drive path failure without weakening its assertion.
At the Gate D boundary, `archie-iris` had 15 unit tests and one compile-fail privacy test,
and `archie-workstation` had 18 passing tests plus one explicitly ignored hardware test.
Both crates passed Clippy with warnings denied.

## Reusable components

- `archie-cerebra`, `archie-policy` and the policy leases preserve the authority order.
- `archie-router` and `archie-model-runtime` already enforce locality/privacy and bounded,
  redirect-free model transport.
- `archie-memory::admit_learning` remains the only route from observations to training.
- `archie-scheduler` owns placement, preemption and checkpoint obligations.
- `archie-workstation` owns explicit WSL/host operations and the FreeCAD execution seam.
- `archie-vault::ContextCapsule` is deliberately non-serializable and is the only approved
  path for private model context.
- Sentry supplies bounded ephemeral frame acquisition, optional GStreamer/ONNX seams,
  consent-bound cameras and anonymous local tracking.
- IRIS Gates B-D now supply revisioned scene state, fixed-point transforms, semantic ROI
  expansion and source-first browser/desktop/camera/image/CAD adapters.

## Training and cloud baseline

There was no Visual Reflex checkpoint, model registry, streaming data reader, sealed-eval
split, cache governor, Nebius job runner, champion/challenger gate or autonomous experiment
controller. `training/datasets.toml` and `docs/TRAINING-DATASETS.md` are admission research,
not an operational trainer. The existing Nebius integration is an inference provider; it
does not provision GPUs or storage.

The available builder has Python 3.14 and no PyTorch or visible NVIDIA GPU. It has about
940 GiB free internally, while Windows C: has roughly 2 GiB free. Builds, caches and model
artifacts therefore belong inside `ArchieOS-Builder`. A dependency-free CPU baseline can
prove the pipeline and interfaces; it cannot establish production vision accuracy or the
30-80M target architecture.

`PentaVM/archie-iris` exists as a private, empty GitHub repository. The trainable Foundry
microproject will live there. ArchieOS will consume only a versioned model manifest and
checkpoint through the replaceable IRIS runtime boundary.

## Integration map

1. Sources emit structured deltas and optional ephemeral frames into the scene graph.
2. Specialists and Visual Reflex advise on bounded ROIs; they cannot execute or persist
   sensory content.
3. Working Scene Memory retains semantic state under fixed TTL/capacity bounds.
4. ACP compiles task-selected node state into checkpoints/deltas for Cerebra.
5. Cerebra proposes semantic actions; policy leases authorize; Motor resolves current
   coordinates and verifies the resulting scene revision.
6. Foundry streams only registry-admitted data, trains challengers and emits signed/hashable
   manifests. Promotion remains an explicit deterministic gate.
7. Forge may vary allowlisted experiment parameters in isolated runs. It cannot change
   privacy, license, authority, promotion or budget ceilings.

## Known blockers

- No production AT-SPI/UIA/Wayland damage bridge, browser driver, Blender runtime, camera
  device, OCR engine, grounding model, pose model or biometric provider is installed.
- No approved real labelled Visual Reflex corpus or sealed benchmark is present.
- No Nebius training credential, project/storage identifiers or spend ceiling is configured;
  a cloud job cannot be launched safely or reproducibly yet.
- Raw face/audio/camera example retention remains Nelson's open retention decision. The
  implementation must retain weights and metadata only.
- The imported WSL image still needs the reviewed PostgreSQL/TimescaleDB/pgvector service
  composed before a release image can be accepted.

