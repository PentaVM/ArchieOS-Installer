# IRIS Foundry

Foundry is the separate `PentaVM/archie-iris` training repository. It contains an
allowlisted dataset registry, revision and license gates, streamed row cache, deterministic
fixed-point trainer, checkpoint/resume, INT8 export, sealed evaluation,
champion/challenger promotion, teacher-label admission, hard-case mining and bounded Forge
experiments.

ArchieOS imports no trainer dependency. It accepts only a small deployment manifest and a
hash-matching checkpoint through `FoundryReflexModel`.
