# Visual Cortex

Visual Cortex is an event-driven advisory interface for ambiguous, task-requested ROIs. A
request contains references and bounded keyframes; outputs are proposals and cannot add
scene nodes the task did not request. Privacy and locality are checked before provider use,
so restricted state cannot reach cloud inference.

`LocateAnythingClient` is the first concrete grounding adapter. The router supplies a
`ModelDescriptor`; `provider-model=nvidia/LocateAnything-3B` keeps the provider-facing
name separate from Archie's path-safe model ID. The adapter accepts exactly one requested
scene node, one digest-bound keyframe ROI and a non-serializable `ContextCapsule`. It sends
the capsule rendering and bounded encoded ROI through the OpenAI-compatible multimodal
API, then parses exactly one normalized `<box>` proposal. It cannot authorize or execute
the resulting geometry.

The wire contract is verified against a loopback HTTP server. A second test proves that a
keyframe digest mismatch causes zero connections, and a third does the same for a capsule
from another task. NVIDIA's model card documents this API shape for local vLLM and SGLang
serving, but currently reports that the model is not deployed by any Hugging Face inference
provider. No weights are bundled.

The same strict grounding path was also executed once against the live Nebius Token Factory
API using the vision-capable `google/gemma-3-27b-it` model currently returned by the account
catalog. The test generated a white 256x256 PNG with one red rectangle in memory, sent a
Public capsule naming that target, mapped the normalized answer back to scene coordinates,
and verified the proposed center lay inside the synthetic target. This is an API and mapping
smoke test, not an accuracy measurement. The live test remains ignored by default and needs
an explicitly injected credential.

LocateAnything-3B is a temporary research adapter, not the commercial product model. Its
published weights are restricted to non-commercial research under the NVIDIA License.
The reviewed license snapshot is
`fb9f8b10ea275ea341e02cd2d5ab0f237b6afb80`; see
<https://huggingface.co/nvidia/LocateAnything-3B>.

Qwen3-VL LoRA/QLoRA integration and a real labelled evaluation remain **BLOCKED** on a
selected checkpoint, licensed corpus and GPU budget.
