# Visual Reflex

`VisualReflexModel` is replaceable and `VisualReflexRuntime` validates manifests,
micro-batches ROIs, bounds detections and marks uncertain outputs for escalation. The
deployed `FoundryReflexModel` runs a fixed-point 8→16→4 INT8 adapter whose checkpoint hash
must match its manifest.

The checked-in champion was trained and evaluated only on a declared synthetic corpus. Its
1000/1000 sealed result is pipeline evidence, not real-world accuracy. A deterministic
unknown-object baseline remains available when no checkpoint is configured.
