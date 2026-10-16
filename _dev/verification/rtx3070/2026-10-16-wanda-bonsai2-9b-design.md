# Wanda on the exact Bonsai 2 9B checkpoint — proposed design

Status: revision 2 approved by the owner on2026-10-16, with inline implementation-plan preparation explicitly selected. No full-model training, scored blind benchmark or model promotion has happened. Subsequent bibliography expansion follows the owner's request to cover Carnegie and mentalism broadly, including darker influence tactics, without adding a moral filter.

Companion specifications: [blind social-cognition benchmark](2026-10-16-wanda-social-benchmark-design.md) and [source catalogue](2026-10-16-wanda-reading-catalogue.md). These are review artifacts, not implemented runners or admitted datasets.

Owner amendments2026-10-16: [progressive curriculum and local AI Arena](2026-10-16-wanda-arena-curriculum-design.md), [structured decision_v1 channel](2026-10-16-wanda-cortex-decision-design.md). These amendments do not add dependencies before native extraction/parity/full-path backward and the first real training updates. The128families are an initial development tranche, not a permanent dataset ceiling.

## Intended outcome

Train a local, reloadable Wanda adapter on the owner's RTX3070 (8GiB) using **Ternary-Bonsai-2-9B**, retaining the existing Core/Cerebra/Policy/Vault authority boundaries. Train social recommendations with evidence, uncertainty, alternatives and consequences. It grants no execution or domain authority.

Contract clarification from code inspection during planning: current `core/crates/archie-cerebra/src/advisory.rs` accepts exactly `proposal_id`, `hypothesis_id`, `stance`, `score_permille`, `evidence_refs`, `produced_at_ms`; it does not accept free-form recommendations, expiry or human-review keys. Preserve that existing integration mode. Rich social-decision explanations are a separate supervised/evaluation task format, not new keys sent to Core. The earlier description of an existing richer recommendation JSON was inaccurate; no runtime schema change is implied by this correction.

Target-branch audit: active Wanda PR38 (`codex/wanda-context`, reviewed head72cc261) does also contain a separate richer `archie_wanda::types::Recommendation`, validator and `QueryResponse` used by its voice client. The previous correction applied to the Core six-field advisory, not the absence of Wanda recommendations on that PR. Preserve both contracts. The new channel supplements them with a host-correlated reference, not extra Core fields or an assumed new live endpoint.

The owner explicitly rejected substituting Bonsai1.7B/4B. The base is the existing `D:\ArchieLocal\models\Ternary-Bonsai-2-9B-PTQ1_0.gguf`, SHA256 `3561df7ac7d26a71ffa991ac2db740b10841c3a350aa2676ccd3ff5d428b4e8b`. Qwen3.5 weights are not an interchangeable substitute.

## Recommended approach

Keep the native PTQ1_0 base frozen and train separate rank8, alpha16 LoRA matrices. Call this **LoRA over the native quantized base**, not a verified implementation of canonical NF4 QLoRA. This preserves the compact source checkpoint and avoids silently changing its basis or requantizing it.

The first training target is the final full-attention block's query/value projections, with a frozen preceding network and frozen language-model head. A 9B model need not update every layer to receive a LoRA adapter. This is the mechanical starting point, not the permanent capacity limit. Expand only against development metrics and measured whole-worker memory; the sealed benchmark cannot select targets, ranks or checkpoints.

Proposed expansion ladder: block31 q/v rank8; then the reviewed q/k/v/output projections of block31; then full-attention blocks27 and31; then further full-attention blocks in descending order if their suffixes are supported. Rank16 is a separate experiment, not a simultaneous untracked change. Each stage retains the previous candidate and records target tensor names/shapes, base/split digests, native zero/nonzero parity, quality/calibration/robustness changes and peak allocated/reserved/total GPU memory. Additional blocks require backpropagation through the intervening frozen recurrent/Gated-DeltaNet blocks, not independent stale layer caches. Stop expansion until that path has native parity and a real resource measurement. MLP/Gated-DeltaNet/embedding targets are not silently treated as ordinary attention projections.

Use at most two seeds per development stage initially. Select by paired development results, retaining the smallest stage with a repeatable improvement and no observed domain/calibration/robustness regression. Keep total device use below 7GiB; the worker's admission budget is the smaller of (7GiB minus other measured device use) and (measured available VRAM minus 768MiB). A nonpositive budget rejects the job. Also enforce the existing RAM/disk guard. These are proposed stop limits, not evidence that the full job fits. If no candidate passes, keep the baseline and report the failed experiment.

The existing Prism engine supplies the frozen prefix. A local observer over its pinned evaluation callback extracts only the selected block's activations/tokens for admitted training examples; it is a worker, not another Core. Cache extraction and Torch training run in separate processes/phases so the two CUDA library stacks do not share a process or occupy the GPU concurrently. Cache entries carry base/input/position/split digests and a size cap; sealed evaluation caches are not trainer inputs. The trainable suffix reproduces the exact Qwen35 attention/gating/RMSNorm/RoPE and native PTQ+Hadamard operations. Its zero-adapter output must agree with the pinned native engine before training is enabled. If that parity fails, the job refuses to run.

Adapters apply in the original activation basis. In pinned Prism `build_lora_mm`, the base branch transforms `cur_mm` and optionally applies `w_s`, while the LoRA branch computes `B(A(cur))` from original `cur` and is added afterward. Do not rotate the LoRA input again or scale its delta by `w_s`.

Read-only inspection of pinned `src/models/qwen35.cpp` also identifies three parity traps. First, the query projection is a joint query-plus-gate projection; reproduce its interleaved split, per-head Q/K normalization and gate, not an ordinary query-only linear layer. Second, the graph can narrow final-layer rows to requested outputs and mask captured rows. Extraction must explicitly capture the pre-block residual for every teacher-forced token with token/position/sequence layout, not only the final output token. `l_out` of block30 is after its `build_cvec`; the suffix must reproduce any active control-vector configuration or reject a nondefault one. Third, training block31 recomputes its K/V and causal attention from the cached frozen-prefix activations at every forward pass; baseline native K/V caches must not substitute for trainable suffix state. Test full-sequence versus microbatch/position handling and reset sequence state between examples. These source observations are not proof that the installed callback/export API exposes every needed tensor.

The frozen LM head is still needed for the token loss and gradient into the suffix. Its vocabulary is 248320; dequantizing the whole [4096,248320] head to FP32 would alone consume about4GiB. The implementation must profile an exact chunked head/loss path, preserving the full-vocabulary normalization and suffix gradient, rather than use a tiny sampled-vocabulary loss and imply equivalent training. Chunking itself does not prove the 8GiB full job fits.

Alternatives considered: reconstruct/requantize the exact Bonsai weights into an NF4 Transformers model (more VRAM/conversion complexity; still requires parity), or replace it with a different trainable model (rejected by the owner). Full-network QLoRA is not claimed by this initial suffix-training design.

## Contracts and data

- Input is a versioned, hash-pinned dataset manifest and explicit training config. No raw household audio, screenshots, WhatsApp history, Vault store or work-domain content is automatically admitted.
- Initial owned synthetic Spanish scenarios follow the existing Wanda schema and are explicitly marked synthetic. The first pipeline corpus uses 128 training and 32 development scenario families. Every family includes a clean case, irrelevant-noise variants, a decision-relevant counterfactual and a plausible but unsupported counterexample recommendation. Split by source narrative/causal family, not rows or cosmetic templates. These small development sets do not establish psychological competence. They and every example in the companion spec are excluded from the independent sealed comparison.
- Training targets explain the user's stated objective, observable evidence versus hypotheses, competing explanations, alternative decisions, likely immediate/delayed consequences, relational costs and what new information would change the decision. Store concise decision justifications, not hidden chain-of-thought. Include cases where acting now is justified, waiting or asking is useful, disagreement serves the user's objective, or a familiar persuasion tactic fails. Social inference is not automatically refusal or abstention.
- Noise includes unrelated notifications, verbosity, misspellings, quoted instructions, message order distractions and emotionally loaded but irrelevant details. Counterfactuals change causal facts such as urgency, prior commitments, power/knowledge asymmetry or the user's priority; cosmetic name/order changes should not alter the decision. Labels distinguish intentionally stable decisions from decisions expected to change. Never train a universal 'always empathize', 'always sell' or 'always ask' shortcut.
- Public candidates: GoEmotions (`add492243ff905527e67aeb8b80c082af02207c3`, card Apache2.0), SODA (`fdc848ab0183208ea7808206c91c724414d0a071`, card CC-BY4.0). Dataset/schema/attribution/content review remains a prerequisite to ingestion. SocialIQA is excluded until its missing API licence metadata is resolved from its primary terms. Existing held-out/evaluation-only sets stay out of training.
- Source material is evidence with provenance, not an oracle about another person's internal state. Historical mentalism assertions are attributed claims, not ground-truth labels. Uncertainty is evaluated against available evidence; do not reward confident invention or universal hedging.
- Keep personal and professional datasets/configuration/checkpoints separate. No adapter enables cross-domain access.
- Export a GGUF adapter plus manifest containing base SHA, runtime commit, tokenizer/chat-template identity, exact target names/shapes, rank/alpha, data/split digests and evaluation receipt. Preserve the previous package. Synthetic compatibility probes can never be promoted as Wanda adapters.

Wanda adds no moral classifier, policy judge, ethical reward term or separate refusal filter. It analyzes and recommends; Core/Cortex retains contracts, leases, domain checks and execution authority. Existing schema/evidence checks validate the integration contract, not a new moral policy. Existing upstream model behavior is measured as observed and is not represented as 'zero refusals' without evidence. Scientific accuracy, authority limits and the user's explicit goals remain separate evaluation dimensions.

## Execution and promotion

The training worker needs start/status/cancel/resume commands with atomic checkpoints. It runs one GPU job at a time, with generation/IRIS paused through existing leases/supervision. Before starting, inspect actual VRAM/RAM/free disk; during training, monitor them and stop/checkpoint before exceeding the configured budget. Initial mechanical smoke sequence length128, batch1, rank8; full recommendation examples/evaluation use512tokens after a measured resource pass, rather than truncating a complete JSON target and calling it valid. Stop before free system disk falls below2GiB. Bound the development activation cache to1GiB. Do not infer full-job memory from the72MiB single-projection test.

Record the frozen 9B baseline and freeze the protocol before optimization. Keep sealed prompts, keys, outputs and scores inaccessible to the trainer and checkpoint selection. After development selection, freeze the candidate hash and run the sealed comparison once. Promotion requires existing Wanda structural/authority/domain checks, zero invented evidence/person references on the integration suite, and a measured social-task improvement over the 9B baseline with calibration/robustness reported separately. A tiny synthetic-corpus improvement is labelled development-only. Beating all three competitors is a separate, stronger research claim governed by the companion protocol; it is not inferred from promotion or JSON validity. No automatic push/publication of weights or datasets.

## Acceptance checks

1. Exact checkpoint/metadata/tokenizer/runtime and tensor mapping are reproducible; wrong base/targets/shape/basis are rejected.
2. Full selected suffix at zero LoRA agrees with native engine on the same real token sequences, including attention state/position handling. A single linear-layer algebra check does not satisfy this gate.
3. CUDA backward has finite gradients, only A/B update, and checkpoint/resume preserves optimizer and step state.
4. Reloaded candidate GGUF reproduces the trained suffix's delta in native inference; disabling it restores baseline.
5. Sealed evaluation is not visible to optimization; schema/evidence/domain checks and social decision quality are independent of training loss. Noise invariance, evidence-sensitive counterfactual changes, calibration and novel-family performance have separate receipts. The experiment reports failures and missing competitors, not just successful examples.
6. Peak total GPU/RAM/disk measured on this PC; cancellation and resource-pressure interruption are exercised without killing another app or changing Core authority.

## Separate follow-up deliverables

IRIS uses a different Foundry `.skill` contract and needs its Rust adapter loader ported with Python/Rust parity. Its trainer remains a separate worker; no Wanda adapter is loaded into IRIS.

The self-contained WSL image must carry Core, reviewed database stack, voice/ASR/Piper, IRIS weights/text tower/runtime and Bonsai weights/runtime with no `/mnt/d` dependency. Keep trainer dependencies in a separate worker environment; do not remove the old IRIS venv while Bonsai CUDA symlinks still reference it. Generate operator keys/identity on first boot, not inside a distributable image. GUI/WSL generation remains coordinated with Claude/Xavi; no PR41 merge or release publication is authorized by the quoted context.

Image construction/import testing needs additional disk headroom; C: reached~2GiB free during the compatibility tests and recovered to~4GiB after stopping them, D:~4.9GB. Do not begin a fresh multi-GB staging tree or claim a final image until capacity and a clean build target are resolved.

## Evidence available now

- Native9B CUDA inference: public prompt returned LISTO; native OpenAI-style tool proposal emitted `get_system_status({})`. This is not proof of an installed general execution loop.
- One actual `blk.31.attn_v.weight` projection decoded identically to installed Prism C; Hadamard algebra relative max error2.95e-7; rank8 LoRA synthetic optimization reduced loss3.44e-4→1.83e-6 without changing the base tensor. This is not full9B/Wanda training.
- Native server loaded zero/nonzero probe GGUFs: zero delta0; nonzero top-logprob delta0.02500057; disable/restore delta0. No probe was promoted.
- HP independently confirmed the pinned forward-path reasoning and the absence of an existing dedicated Wanda trainer or published new Rust WSL asset.

Primary references: [Prism9B model card](https://huggingface.co/prism-ml/Ternary-Bonsai-2-9B-gguf), [pinned native graph](https://github.com/PrismML-Eng/llama.cpp/blob/e3de715222b2667c54a267441159a5729f11155e/src/llama-graph.cpp), [PEFT quantization/QLoRA](https://huggingface.co/docs/peft/main/developer_guides/quantization).
