# IRIS repository baseline

## HEAD

The IRIS audit started at `6bed0bd567511b7ad4d9e716d3a3dcf45e2cfd46`, after
rebasing the workstation work onto `origin/main` at `d47508b`. The protected historical
repository at `C:\Users\Penta\Desktop\Archie` was not read or modified.

Relevant recent changes are:

- `d47508b` architecture, dataset and preloaded-tool review;
- `bed8f64` content-free cognitive visualization;
- `c7dec2e` MCP redirect refusal;
- `721e873` bounded Cerebra analytical advisory;
- `73170f6` admitted context-utility learning;
- `96a8a57` local floor estimator;
- the rebased workstation commits implementing clarification, provider/harness adapters,
  local speech contracts, FreeCAD leases and WSL packaging.

## Existing modules reused

- `archie-cerebra` owns sequencing, silence, clarification and advisory interpretation.
  IRIS may publish observations and compact context; it may not decide or execute.
- `archie-policy::Lease` is the only authority for motor/tool effects.
- `archie-router` separates privacy disqualification from model scoring. Visual Cortex
  selection must use it rather than a model name.
- `archie-model-runtime` provides bounded, redirect-free OpenAI-compatible transport.
- `archie-memory::admit_learning` excludes raw images/video/transcripts and admits only
  receipt-backed supervision. IRIS does not create a parallel training path.
- `archie-scheduler` already owns slots, pressure admission, preemption and checkpoint
  obligations. IRIS will publish demand instead of scheduling itself.
- `archie-workstation` provides typed host operations and a policy-scoped FreeCAD executor.
- Sentry already has an ephemeral C++ `FrameSource`, an optional GStreamer/ONNX seam,
  authorized camera connection logic and anonymous per-camera person tracking. It does not
  provide a general UI/application scene graph.
- `archie-observability` already has content-free cognitive traces; IRIS traces will follow
  the same metadata-only rule.
- The WSL distribution disables Windows drive automount and process interop. WSLg access
  must use an explicit host/source contract and must not reopen those boundaries.

## Missing modules

No existing module provides the complete IRIS fabric. The audit found no Rust world scene
graph, semantic damage/ROI compiler, DOM/accessibility adapter, static-image cache, OCR
specialist contract, open-vocabulary grounding contract, event-driven Visual Cortex request,
working scene ring, ACP checkpoint/delta compiler, semantic motor interface or shared-weight
multi-source session runtime. There is also no AT-SPI, UI Automation, compositor damage or
WSLg/PipeWire production adapter.

The optional Sentry GStreamer and ONNX targets are implementation seams, not installed
features unless their dependencies and flags are present. Biometrics, pose, Blender and
remote perception likewise have policy/interface foundations only.

## Contracts preserved

1. IRIS sees; Cerebra understands; policy authorizes; the executor acts.
2. Structured DOM, accessibility and application object state take priority over pixels.
3. Raw screen/camera/image buffers remain bounded, ephemeral and non-serializable.
4. Models cannot grant leases, promote durable memory, select evidence or weaken privacy.
5. Remote inference receives only task-selected ROI/keyframe references and sanitized ACP,
   after the existing router and gateway checks.
6. Node identifiers and current transforms resolve coordinates at execution time.
7. The existing scheduler remains the sole placement/preemption authority.

## Tests before IRIS changes

On Arch Linux in the WSL builder, with `CARGO_INCREMENTAL=0` and one build job, the audit
ran the relevant Rust packages at this HEAD. It recorded 394 passing tests and one failure:

- Cerebra 41; memory 132; model runtime 9; policy 32; router 33; scheduler 38;
- speech 92; workstation 17 passing and 1 failing.

`archie-workstation::host::bridge_rejects_host_and_parent_paths` failed because a Windows
drive-qualified path (`C:\secret`) is treated as an ordinary relative component by
`std::path::Path` on Linux. This is a real cross-platform boundary defect and will be fixed
without changing the assertion.

The earlier pre-workstation baseline on Windows recorded 681 passing tests across Cerebra,
memory, observability, Vault, speech, router, policy, execution, model runtime, CLI,
installer and `archied`. The remote display commit adds C++ tests; those remain a separate
gate because they require the CMake toolchain.

## Known blockers

- The imported WSL image boots, disables Windows automount and runs FreeCAD, but its first
  test exposed that `archie-core.service` used a simulation root and omitted ArchieOS's
  PostgreSQL/TimescaleDB/pgvector services. The release artifact is not accepted until the
  existing reviewed database stack is composed and boot-tested.
- No labelled Visual Reflex dataset or trained 30–80M checkpoint is present. v0.1 therefore
  needs a real replaceable interface and an explicitly named deterministic baseline; it
  cannot claim neural accuracy.
- No configured target-model tokenizer is available locally. ACP benchmarks must label
  byte/structure measurements separately and refuse model-token claims unless an actual
  tokenizer adapter is configured.
- No camera, biometric enrolment, WSLg compositor-damage API, AT-SPI service or Blender
  runtime is available on this test host. Hardware/application adapters must report this
  state rather than simulate operational support.
- Raw biometric or labelled camera retention still requires Nelson's explicit retention
  decision. IRIS will retain no such corpus.

