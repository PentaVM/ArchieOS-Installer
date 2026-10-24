# Exact Bonsai2 9B Wanda Trainer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task by task, inline in this chat. Steps use checkbox syntax. The owner selected inline execution; no delegation is selected.

**Goal:** Produce a resumable, locally measured Wanda LoRA candidate over the exact existing Bonsai2 9B PTQ1_0 base, with source receipts and native inference parity.

**Architecture:** Native Prism extracts the frozen prefix in a separate process; a Torch worker trains a faithful final-block suffix and frozen, chunked full-vocabulary head. Data admission, resource supervision, candidate export and development evaluation are separate modules. Core remains the authority and is not replaced by the trainer.

**Tech Stack:** Python3.14.8, installed Torch2.11.0+cu128/CUDA12.8 and NumPy for the first measured compatibility run; C++17 shim linked against installed Prism b10777/e3de715222b2667c54a267441159a5729f11155e. Runtime CUDA12.4 and TorchCUDA12.8 remain in separate processes. Python standard-library unittest for CPU/control tests, separate opt-in hardware commands. Isolated production dependency packaging follows a disk-capacity check; no duplicate multi-GB environment installation as a planning step.

**Spec:** [approved trainer design](2026-10-16-wanda-bonsai2-9b-design.md), [reading catalogue](2026-10-16-wanda-reading-catalogue.md). [Benchmark implementation](2026-10-16-wanda-benchmark-plan.md) is an independent plan; sealed evaluation is not required to test trainer mechanics.

**Owner amendments:** [progressive curriculum/local Arena](2026-10-16-wanda-arena-curriculum-design.md) and [Cortex decision channel](2026-10-16-wanda-cortex-decision-design.md). Target publication is existing Wanda PR38, preserving its Recommendation/QueryResponse as well as Core's six-field advisory.

Status: amended with the owner's three2026-10-16 requests for publication/review on PR38. The owner approved design revision2 and selected inline work. All implementation tasks below remain incomplete. Source inspection and earlier compatibility probes are not task completion evidence.

**Execution order:** Tasks1–4 → Task5A minimal owned social shard → Task6 real updates/checkpoint and Task7 native reload → initial bounded semantic run. Task5B corpus expansion, the Arena plan and Task9 Cortex transport are independent later work. Do not wait for128families, full books, Claude Code installation, remote access or a Cortex/GUI/WSL deployment to prove parity/backward and the initial real updates.

## Global Constraints

- Verify origin `PentaVM/ArchieOS-Nvidia` before editing/integration. Preserve other worktrees and Claude/Xavi GUI/WSL generation files; no PR41 merge or release work here.
- Base SHA256 `3561df7ac7d26a71ffa991ac2db740b10841c3a350aa2676ccd3ff5d428b4e8b`; no different Bonsai, Qwen base or NF4 substitution. Name the method native-quantized-base LoRA, not validated canonical QLoRA.
- Start `blk.31.attn_q.weight` (joint query/gate) and `blk.31.attn_v.weight`, rank8/alpha16, frozen base. Additional targets are development experiments, never sealed-set selection.
- Preserve Core/Cortex/Policy/leases, evidence/domain admission and voice priority. No additional moral judge, ethical reward, refusal layer or topic ban is added to Wanda.
- Exactly six current Core advisory fields: `proposal_id`, `hypothesis_id`, `stance`, `score_permille`, `evidence_refs`, `produced_at_ms`. Rich strategic examples use a separate task format, never additional Core keys.
- No raw chats/Vault/audio/screenshots admitted automatically. Personal/work domains remain separate. Book rights/provenance and evidence quality are data properties, not moral classifications.
- Full run limit512tokens, batch1; mechanical smoke128tokens. Reject incomplete target truncation. Cache cap1GiB; system disk free floor2GiB; total GPU ceiling7GiB; worker budget min(7GiB-other use, available VRAM-768MiB). Unknown readings reject admission.
- Initial use is manual under the existing authorized supervisor with inference/IRIS idle; no killing or pausing unrelated apps. Do not claim an OS scheduling lease exists because a worker has a flock. Automated REM deployment is a separately verified integration, not a new authority path.
- No semantic training until real native suffix/head parity, backward and interruption gates pass. No candidate promotion from synthetic compatibility probes.

## Review Focus

1. Final-token-only native capture can silently discard teacher-forced targets; Task2 proves full-row, position and microbatch coverage.
2. Prefix activations cannot replace suffix K/V after LoRA changes; Task3 recomputes suffix state and tests nonzero deltas across tokens.
3. Chunked head can produce plausible loss with wrong normalization/gradient; Task4 compares full-vocabulary loss and input gradient against an independent dense reference.
4. Book aliases/translations and causal variants can leak across splits; Task5 groups ancestry and rejects overlap, without moral topic filtering.
5. Stale PIDs, absent telemetry or partial checkpoints must not kill another app or resume corrupt work; Task6 uses ownership checks, atomic state and fail-closed resource admission.

## File structure and shared records

Create a self-contained Python project `training/wanda_bonsai2/` with `pyproject.toml`, `README.md`, `src/archie_wanda/` and `tests/`. New production modules must not import `_dev/probes`; those remain labelled throwaway references.

- `contracts.py`: immutable `ModelIdentity`, `TrainConfig`, `SourceReceipt`, `Example`, `TokenExample`, `CacheReceipt`, `ParityReceipt`, `RunReceipt`, `CandidateReceipt`, and typed `AdmissionError`/`ParityError`/`ResourceError`. All receipts carry `format_version=1` and canonical-JSON SHA256. Tensor payloads live in bounded files, not JSON arrays. `resources.py` defines ResourceSnapshot; `development.py` defines DevelopmentReceipt and StageDecision.
- `model.py`, `ptq.py`: read-only GGUF directory, model identity, tensor chunks and frozen quantized linear operations.
- `native/CMakeLists.txt`, `native/prefix_export.cpp`, `native.py`: pinned shim, extraction process and native reference outputs. No source modifications inside the installed Prism directory.
- `suffix.py`, `head_loss.py`: differentiable block31 and exact chunked cross-entropy head.
- `sources.py`, `data.py`: bibliography admission/fetch receipts, causal-family splits, task rendering/token masking.
- `resources.py`, `checkpoints.py`, `worker.py`, `cli.py`: bounded local execution and lifecycle; console script `archie-wanda-train=archie_wanda.cli:main`.
- `export.py`, `development.py`: GGUF adapter export/native reload receipts and development-only stage comparison.
- `decision.py`: typed decision_v1 parser/receipt writer; later Core-facing changes belong to Task9 on the actual Wanda branch, not a replacement parser in the trainer.
- `configs/rtx3070-smoke.toml`, `configs/rtx3070-development.toml`, `sources/reading-candidates.json`: reviewed configuration and candidate metadata. Runtime corpus/cache/checkpoints stay outside Git.

Shared record details: `ModelIdentity` includes base/file size/hash, Prism commit, architecture, tensor shapes/types, tokenizer/template digests and active control-vector identity. `Example` includes id, source/domain, causal-family id, variant, task mode, admitted evidence ids, messages/target and source-receipt digests. `CacheReceipt` adds exact input tokens/positions/sequence layout, captured layer/row count and extraction settings. `TrainConfig` includes seeds, targets, optimizer settings, sequence/step/time limits and resource ceilings. Receipts cannot contain API credentials, raw private source material or fabricated model hashes.

All CPU test commands below run from `training/wanda_bonsai2` using `PYTHONPATH=src python -m unittest ...` in Linux. Tests requiring the real GPU/base use the explicit `archie-wanda-train verify-*` commands and create new timestamped receipts under `_dev/verification/rtx3070/results/`. Hardware commands are future interfaces, not currently installed commands.

## Task1: Exact identity and frozen PTQ operators

**Files:** contracts.py, model.py, ptq.py, cli.py, resources.py (initial preflight/ResourceSnapshot); tests/test_identity.py, tests/test_ptq.py; pyproject.toml, configs/rtx3070-smoke.toml. Initial CLI implements `inspect`; verification subcommands are added by their owning tasks, not placeholder successful commands. Every hardware verification command reuses the resource preflight before allocating/loading, rather than waiting until Task6 for those guards.

**Interfaces:** `inspect_model(path: Path, expected_sha: str) -> ModelIdentity`; `read_tensor_rows(model: ModelIdentity, name: str, first: int, count: int) -> numpy.ndarray`; `FrozenPTQLinear.forward(x: Tensor) -> Tensor`. The latter streams dequantized chunks, exposes input gradients, and never registers base weights as trainable parameters.

- [ ] Write failing tests for wrong base, unsupported tensor type, truncated/overlapping ranges and changed file between identity check/read. Test PTQ type143 group128/28byte decoding against pinned C dequantization, including tails and scale signs. Verify sign/Hadamard basis and original-activation LoRA addition.
- [ ] Run `PYTHONPATH=src python -m unittest tests.test_identity tests.test_ptq -v`; confirm failures for missing implementation, then implement the read-only parser/operators. Reject unsupported metadata rather than guessing a Qwen layout.
- [ ] Re-run CPU tests. Run a real single-projection forward/input-gradient comparison against a dense reference and installed Prism C unpacker; record base unchanged, shapes and tolerances. Earlier random one-layer probes are reference evidence, not a replacement for these maintained tests.
- [ ] Review and commit only these files plus their receipt; preserve all unrelated changes. Do not install a CUDA toolkit or rewrite the base.

## Task2: Complete native prefix extraction and native reference

**Files:** native/CMakeLists.txt, native/prefix_export.cpp, native.py, tests/hardware/public-parity.jsonl; tests/test_native_cache.py; extend cli.py with `verify-extraction`.

**Interfaces:** native executable `archie-wanda-prefix --model FILE --request FILE --out DIR`; `extract_prefix(model: ModelIdentity, examples: Sequence[Example], out: Path) -> CacheReceipt`. Request version1 contains explicit token ids, text positions, sequence ids, block index31, admitted split and settings. Output contains block30 post-control-vector residual for every token, selected native intermediate reference tensors and loss-position logits needed for parity. No prompt evaluation from an arbitrary URL.

- [ ] Write failing tests for cache cap, row-count/layout mismatch, final-token-only extraction, state carried across examples, unsupported control vectors and split identity mismatch. Use small reference fixtures for file-contract tests only.
- [ ] Implement the shim against exact pinned headers and installed shared libraries. Verify ABI/runtime fingerprint before creating a context. Prefer the existing staging `llama_set_capture_layers`/`llama_get_embeddings_capture_ith` tap for layer30, after confirming its all-requested-row semantics. Installed `nm -D` really exports these as C++-mangled symbols; pinned llama-context.cpp defines them but the fetched public llama.h does not declare them. Declare the exact pinned free-function signatures in the C++ shim, outside extern-C, rather than guess a stable C ABI or change the installed runtime. If unusable, use the documented eval callback with computed backend copies and correct ne/nb/dtype ownership. If neither path exposes all needed rows, record capability failure and stop; no substitute activations. Reset state per example and request all required output rows. Keep the binary in the trainer's directory.
- [ ] Run `PYTHONPATH=src python -m unittest tests.test_native_cache -v`. Run `archie-wanda-train verify-extraction --config configs/rtx3070-smoke.toml --cases tests/hardware/public-parity.jsonl` on at least three public text sequences with mixed lengths. Compare one prefill versus multiple microbatches, position handling and repeated clean sequence reset. Record actual CPU/GPU memory and full native settings.
- [ ] Commit shim, tests and receipt only after complete token/row coverage passes. Extraction cannot start a Torch GPU worker concurrently. Do not claim an HTTP llama-server endpoint exposes hidden states unless it actually does.

## Task3: Faithful final-block suffix and LoRA gradients

**Files:** suffix.py, tests/test_suffix.py; reuse tests/hardware/public-parity.jsonl and extend cli.py with `verify-suffix`.

**Interfaces:** `BonsaiSuffix(model: ModelIdentity, config: TrainConfig)`; `forward(hidden: Tensor, positions: Tensor, sequence_layout: Tensor) -> Tensor`. Input is full frozen-prefix activations; output is final normalized hidden states. Trainable names must be only the configured A/B tensors.

- [ ] Write failing tests for joint query/gate interleaving, Q/K RMS normalization, exact rotary sections, causal/GQA masks, gate, residual/FFN and per-sequence reset. Include zero adapter, nonzero adapter, prompt-only prefix plus teacher-forced continuation and batch-boundary consistency. Assert base hashes unchanged and expected finite A/B gradients after two steps (zero-initialized B can give A zero gradient on the first step).
- [ ] Implement frozen chunked linear operations plus rank8/alpha16 deltas in the original activation basis. Recompute suffix K/V on every forward; never reuse baseline suffix caches. Start FP32 reference math; reduced-precision paths require their own parity receipts.
- [ ] Run `PYTHONPATH=src python -m unittest tests.test_suffix -v` and `archie-wanda-train verify-suffix --config configs/rtx3070-smoke.toml`. Predeclare normalized RMS error<=5e-4 against native hidden states and full-logit KL<=1e-3 on the public parity corpus; log maxima, native repeat variability and failures. Do not loosen thresholds after observing a failed run merely to permit training. Nonzero deltas must agree in sign/shape and return to baseline on disable.
- [ ] Commit only after real zero/nonzero suffix parity and finite backward pass. If ABI or numerical parity fails, later semantic tasks remain disabled; a successful sampled linear layer is insufficient.

## Task4: Full-vocabulary bounded head loss

**Files:** head_loss.py, tests/test_head_loss.py; extend cli.py with `verify-loss`.

**Interfaces:** `full_vocab_cross_entropy(hidden: Tensor, labels: Tensor, head: FrozenPTQLinear, vocab_chunk: int = 2048) -> Tensor`. Labels use -100 to mask prompt/padding positions; compute normalization over all248320 vocabulary entries. Custom backward replays bounded weight chunks and returns exact hidden gradients without retaining the whole dequantized head or all chunk graphs.

- [ ] Write failing tests comparing scalar loss and hidden gradient to independent dense PyTorch cross-entropy on multiple small vocabularies/chunk sizes, ignored labels, extreme logits and a final partial chunk. FP64 reference tolerance1e-8 for FP64 path; FP32 relative loss/gradient tolerance1e-5. All-ignored rows reject the example rather than return a fake successful update.
- [ ] Implement streaming log-sum-exp and target-logit accumulation plus chunk replay backward. Keep the full quantized head frozen and read-only; no sampled-class approximation. Provide a dense small-model reference used only in tests.
- [ ] Run `PYTHONPATH=src python -m unittest tests.test_head_loss -v`; then `archie-wanda-train verify-loss --config configs/rtx3070-smoke.toml`. Measure actual full9B head+suffix backward peak memory/time for seq128, then separately512 only if the resource budget permits. Return an incomplete capability receipt if it does not fit.
- [ ] Commit operators and real resource receipt. This is a full-path mechanical backward proof, not evidence of social improvement.

## Task5: Broad-source corpus admission and social examples

**Files:** sources.py, data.py, sources/reading-candidates.json, tests/test_sources.py, tests/test_data.py; modify training/datasets.toml only for actually admitted datasets.

**Interfaces:** `admit_source(record: dict, rights_receipt: Path) -> SourceReceipt`; `fetch_source(receipt: SourceReceipt, destination: Path, max_bytes: int) -> SourceReceipt`; `load_examples(manifest: Path, split: Literal['train','development']) -> list[Example]`; `render_example(example: Example, model: ModelIdentity, max_tokens: int) -> TokenExample`. `TokenExample` contains token ids, positions, labels, family/domain and receipt digests. Sealed manifests are not accepted by the trainer API.

- [ ] Write failing tests for candidate-versus-admitted rights, changed edition/hash, author alias/translated duplicate overlap, causal family split crossing, personal/work mixing, forged citations, full-target truncation and quoted instructions. Include sources on dark influence as normal provenance-labelled content; assert there is no moral-topic admission field. Unknown rights leave that source pending without blocking admitted project-owned data.
- [ ] Transfer the expanded catalogue as candidate metadata with stable work/edition/author identifiers; no invented full-text URLs. Implement bounded HTTPS fetch only for admitted exact source URLs with byte/hash receipts and retained notices, or local authorized files. Do not auto-purchase/scrape commercial books. Default download batch budget256MiB with resumable progress; no arbitrary lifetime book-count limit.
- [ ] **Task5A, first-update shard:** admit8owned Spanish social families/short complete targets using the same source/domain/split/target checks. Four are train, four development; every family has clean/noise/causal-change/counterexample variants, plus cosmetic invariance examples. Human-reviewed concise targets preserve user goals/evidence/alternatives/consequences/uncertainty. This small shard enables Task6 actual updates and native reload before completing the larger corpus; it cannot support a competence or superiority claim.
- [ ] **Task5B, independent expansion:** extend to128 train/32 development causal families, then review additional causal-diversity tranches without a permanent volume ceiling, as specified in the curriculum amendment. Store decision_v1 concise justifications separately from advisory_v1 six-key targets. Human-authored/reviewed targets and facts, source/claim provenance, alternatives, consequences and user goals are explicit. Include success/failure of techniques and cases favoring action, waiting or clarification. Do not generate a blanket refusal or agreeable-tone target. Keep source-derived families together; balance sampling by author/work so duplicated Atkinson/Carnegie editions cannot dominate solely through volume. Record a causal-coverage ledger and tranche ablations, including failed/uncertain gains.
- [ ] Run `PYTHONPATH=src python -m unittest tests.test_sources tests.test_data -v`. Validate all full targets fit the512-token training limit or reject/shorten by explicit editing with a new hash. Record actual admitted source/example counts and attribution, distinct from51 catalogue titles. Commit owned dataset/admission receipts only; no private text or sealed cases.

## Task6: Resource-aware worker, checkpoints and interruption

**Files:** extend resources.py with continuous worker supervision; create checkpoints.py, worker.py; extend cli.py lifecycle commands and configs/rtx3070-smoke.toml; create configs/rtx3070-development.toml and tests/test_worker.py.

**Interfaces:** `preflight(config: TrainConfig) -> ResourceSnapshot`; `train(config: TrainConfig, manifest: Path, run_dir: Path, resume: Path | None) -> RunReceipt`; CLI `start`, `status`, `checkpoint`, `cancel`, `resume`, `verify-extraction`, `verify-suffix`, `verify-loss`. State transitions: queued→extracting→training→checkpointed/completed/failed/cancelled. `checkpoint` stops safely with resumable state; `cancel` writes a terminal receipt. `status` reads metadata without allocating GPU memory.

- [ ] Write failing tests for nonpositive budget, missing/stale telemetry, another active owned run, unowned/reused PID, interrupted atomic rename, mismatched dataset/base/optimizer resume, user-return activity and disk pressure. No shell-built commands; no broad process termination. Checkpoint only at a consistent gradient-accumulation boundary or save the accumulation state explicitly.
- [ ] Implement a per-user flock and PID/start-time/boot-id/UID/executable ownership checks, independent of Core authority leases. Before GPU allocation require the existing supervisor's ownership/admission or verify inference is stopped and no conflicting job is active; otherwise refuse with a concrete dependency. Monitor total device memory, /proc/meminfo and both guest root/hostC: free space every step and before writes. Read the supervisor activity signal when supplied; stop/checkpoint on return or stale activity. Never fabricate idle from absent telemetry.
- [ ] Write checkpoints atomically in the same filesystem with A/B, optimizer, step/accumulation, seeds/RNG, config/data/base hashes and version. Read tensor checkpoints with `weights_only=True`; no arbitrary pickle/import hooks. Heartbeat every completed step. Smoke: two real updates, batch1/seq128. First bounded development schedule: AdamW lr1e-4, betas(0.9,0.999), eps1e-8, weight_decay0, grad_clip1, dropout0, accumulation8, three epochs/max192optimizer steps, max60minutes, seed17; seed29 is a separate run. These are initial development settings, not guaranteed optimal settings.
- [ ] Run `PYTHONPATH=src python -m unittest tests.test_worker -v`. On the real9B path exercise checkpoint/resume equivalence and owned-worker cancellation, record resource peaks and stop reason. When packaging later integrates REM, reuse `archie-rem::jobs::Job` checkpoint/resume/heartbeat argv contract; do not overwrite its existing trainer or implicitly read its private day windows. Commit worker and hardware receipt after passing.

## Task7: Export, native reload and existing Core contract

**Files:** export.py; tests/test_export.py, tests/hardware/public-advisory.jsonl; README.md.

**Interfaces:** `export_candidate(run: RunReceipt, destination: Path) -> CandidateReceipt`; `verify_native_candidate(candidate: CandidateReceipt, cases: Path) -> ParityReceipt`. Export GGUF A/B plus sidecar with rank/alpha, exact names/shapes, runtime/tokenizer/base/data/checkpoint hashes and development result identity.

- [ ] Write failing tests for wrong target basis/order, wrong base, unsupported layers, changed tokenizer/template, missing receipt and an artifact named PROBE. Refuse overwrite/promotion of an active package. A model-generated checkpoint hash is never trusted.
- [ ] Implement atomic versioned candidate export using pinned Prism GGUF conventions. Start a temporary authenticated loopback native process in a separate GPU phase; do not change `current.gguf` or the normal8081 service. Load candidate, exercise nonzero delta, disable and verify restoration; clean up only that owned process.
- [ ] Run `PYTHONPATH=src python -m unittest tests.test_export -v` and `archie-wanda-train verify-native --candidate PATH --cases tests/hardware/public-advisory.jsonl`. Compare native candidate with the trained suffix on real input, then run existing `cargo test -p archie-cerebra --test advisory --test wanda_bridge` from core using an existing build cache only if disk budget permits. New rich decision JSON must not be sent to the six-key Core parser. Record any unrun Rust build as unverified, not passed.
- [ ] Commit exporter and receipts. Export produces an unpromoted candidate; use an explicit reviewed promotion command later, preserving rollback. No release upload.

## Task8: Development comparison and gradual expansion

**Files:** development.py; tests/test_development.py; stage configs under configs/; README.md.

**Interfaces:** `compare_development(baseline: RunReceipt, candidate: CandidateReceipt, manifest: Path) -> DevelopmentReceipt`; `propose_next_stage(receipts: Sequence[DevelopmentReceipt], resources: ResourceSnapshot) -> StageDecision`. Receipt carries per-dimension evidence, noise/counterfactual outcomes, calibration where labels resolve, target stage/seed and metrics, not an opaque social-IQ scalar.

Task8 also defines `review_data_tranche(anchor: DevelopmentReceipt, incoming: Sequence[Example]) -> TrancheDecision` and TrancheDecision in development.py. Decisions include causal novelty, annotation consistency, overlap, anchored-versus-fresh development results, two-seed ablation uncertainty and admit/revise/defer reasons. A metric plateau alone cannot authorize data or layer growth. Sealed content/results never become inputs.

- [ ] Write failing tests rejecting sealed inputs, mixed bases/conditions, scoring only JSON validity, pseudo-independent variants, translated/cosmetic 'new' families and automatic layer expansion without parity/memory evidence. Hold the development anchor fixed in the regression test so replacing cases cannot inflate gain. Reject a stage that improves mean decision quality while worsening measured calibration or robustness. Missing human strategic ratings remain missing.
- [ ] Implement development-only paired reporting versus unchanged9B, then the reviewed stage ladder: block31q/v→block31q/k/v/output→blocks27and31→additional supported full-attention blocks; rank16 is a separate axis. Stage2 reuses the parity tests. Stage3+ is deliberately disabled until differentiable intervening recurrent blocks and their native state resets are implemented/proven in a follow-up reviewed slice; independent per-layer caches are not a workaround.
- [ ] Run `PYTHONPATH=src python -m unittest tests.test_development -v`; perform one bounded actual development run only after Tasks1–7 gates and source admission. Record training loss separately from decision evidence and resource measurements. Candidate selection uses development only; freeze selected hashes for the independent benchmark plan.
- [ ] Commit development results, including failures. End with exact installed/trained/verified capability labels. IRIS Rust skill loader, WSL image generation, GUI changes and voice-driven arbitrary execution remain separate deliverables under existing ownership.

## Task9: Independent structured decision context for Cortex

**Files on the Wanda target branch:** create `core/crates/archie-wanda/src/decision.rs`, `core/crates/archie-wanda/tests/decision.rs`, `protocols/wanda-decision.schema.json`; narrowly extend Wanda store/runtime/server for negotiated decision-reference read access. Preserve `advisory.rs`, Recommendation/QueryResponse schemas and existing Cortex wire fixtures. Add a domain-scoped read-only projection test at the existing trusted graph bridge, not a new authority service. This task is independent of the first training run.

**Interfaces:** `parse_decision(text: &str, admitted: &DecisionContext) -> Result<DecisionAnalysis, WandaError>` validates the model content; `store_decision(context: &DecisionContext, analysis: DecisionAnalysis) -> Result<DecisionReceipt, WandaError>` assigns host identity/correlation/expiry and stores the inferred record. Define DecisionContext/DecisionAnalysis/DecisionReceipt in decision.rs using the exact fields/bounds in the channel specification. New read clients negotiate `wanda.decision_v1`; old clients retain their old responses. No field is appended to the six-field Core JSON.

- [ ] Write failing tests for strategic round trip, duplicate/unknown alternatives, nonfinite probability,16KiB aggregate cap, expired or cross-domain references, forged host correlation, unsupported version and inference counted as independent evidence. Run old six-field and PR38 Recommendation/QueryResponse fixtures unchanged.
- [ ] Implement typed parsing/storage/correlation using existing Vault domain.reference and local-only authenticated transport conventions. Project only a bounded inferred Document reference through the trusted graph bridge; retrieve via existing Cortex context requests. Do not add actions/permissions/policy overrides or moral classifications. A missing consumer reports unavailable without breaking existing voice/advisory operation.
- [ ] Run `cargo test -p archie-wanda --test decision --test advice --test voice_contract` and existing Cerebra advisory/Cortex wire compatibility suites on the target branch. Separate structural test success from actual local query→Vault→Cortex retrieval→Core-gate integration; record what really ran.
- [ ] Commit compatible channel/tests and receipt separately after first training mechanics, preserving rollback and PR38 collaborators' edits. No GUI/WSL ownership changes.

## Self-review and handoff

Coverage: exact base/basis/quantization Tasks1–4; source breadth/no moral filter/data separation Task5A/B; compute lifecycle Task6; native deployment and Core authority Task7; social evaluation/data and capacity expansion Task8; compatible strategic context Task9 plus independent Arena/benchmark plan. Main unresolved feasibility questions are installed native capture ABI, faithful recurrent suffix expansion and measured full-path8GiB memory. Those are explicit real-test gates, not assumed solved by this plan.

Dependency self-review: smoke config/identity/CLI enter in Task1; verification subcommands arrive with Tasks2–4; the public parity corpus enters in Task2 before its first use. Lifecycle commands arrive in Task6. No task relies on a later task's file for its acceptance command. HP messages38–40 supplied read-only source review; local source/symbol checks confirmed the capture definitions and C++ linkage, not a successful activation extraction.

Execution remains inline, with the amended plan published on the existing PR for review. No new choice of base or execution method is needed. The first implementation deliverable is native parity and full-path backward followed by real updates on Task5A; corpus growth, Arena adapters and Task9 must not delay it. Accumulating more books is not allowed to conceal a failed training path.
