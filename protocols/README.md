# protocols

**Status: IMPL**

JSON Schemas for every object that crosses a trust or process boundary. Language-independent source of truth.

`cortex-correction-trajectory.schema.json` is the cross-language contract for offline
correction parsers and replay tooling. It carries only opaque vault/provenance references,
graph-writer receipts and SHA-256 digests; raw private payloads and any authority to mutate
the graph are deliberately excluded.
