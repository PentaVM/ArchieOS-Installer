# Round 2 exact Bonsai2 trainer audit and strict reference

2026-10-16. Origin verified `https://github.com/PentaVM/ArchieOS-Nvidia.git` before
changes. Claude remains main developer. Initial audit was CPU-only in the existing
training worktree. Root then authorized a new managed worktree at
`C:\Users\Penta\.codex\worktrees\wanda-strict-reference\ArchieOS`, starting `16841cf`,
and two short, separately guarded numerical GPU experiments. No commits, pushes,
queue/control/watchdog edits, base changes, semantic training or promotion occurred
in this audit phase. This method remains native-quantized-base LoRA, not canonical
NF4 QLoRA.

## Numerical finding and real results

The existing FP32 suffix multiplies dequantized PTQ weights by FP32 activations.
Pinned native CUDA normally quantizes activations to Q8_1 before its PTQ matvec/MMQ
kernels. These are different forward operators even when PTQ weight decoding is
identical. Moreover, pinned native cuBLAS enables TF32 math by default. Native
layout variation is therefore diagnostic evidence of kernel changes, not a reason
to replace the predeclared suffix RMS tolerance after a failure.

Exact pinned source paths, read from upstream because the local bundle lacked CUDA
implementation files:

- [PTQ matvec dispatch, lines295–305](https://github.com/PrismML-Eng/llama.cpp/blob/e3de715222b2667c54a267441159a5729f11155e/ggml/src/ggml-cuda/mmvq.cu#L295):
  NVIDIA Turing and later use PTQ MMVQ for at most four token columns.
- [PTQ MMQ selection, lines366–435](https://github.com/PrismML-Eng/llama.cpp/blob/e3de715222b2667c54a267441159a5729f11155e/ggml/src/ggml-cuda/mmq.cu#L366):
  normal PTQ MMQ covers larger batches; `GGML_CUDA_PTQ1_0_MMQ_MAX_BATCH=0`
  disables that choice. It does not disable the earlier MMVQ choice.
- [CUDA dispatch, lines1822–1883](https://github.com/PrismML-Eng/llama.cpp/blob/e3de715222b2667c54a267441159a5729f11155e/ggml/src/ggml-cuda/ggml-cuda.cu#L1822):
  after MMVQ/MMQ, the graph falls through to cuBLAS.
- [cuBLAS conversions and SGEMM, lines1414–1554](https://github.com/PrismML-Eng/llama.cpp/blob/e3de715222b2667c54a267441159a5729f11155e/ggml/src/ggml-cuda/ggml-cuda.cu#L1414),
  [compute override, lines1627–1667](https://github.com/PrismML-Eng/llama.cpp/blob/e3de715222b2667c54a267441159a5729f11155e/ggml/src/ggml-cuda/ggml-cuda.cu#L1627):
  `GGML_CUDA_CUBLAS_COMPUTE_TYPE=f32` converts quantized weights and input
  activations to F32 and selects SGEMM. A compute override alone does not override
  earlier quantized dispatch.
- [cuBLAS handle setup](https://github.com/PrismML-Eng/llama.cpp/blob/e3de715222b2667c54a267441159a5729f11155e/ggml/src/ggml-cuda/common.cuh):
  `CUBLAS_TF32_TENSOR_OP_MATH` is enabled. Child-only
  `NVIDIA_TF32_OVERRIDE=0` is also required for the controlled FP32 reference.
- [Q8 activation quantizer, lines70–127](https://github.com/PrismML-Eng/llama.cpp/blob/e3de715222b2667c54a267441159a5729f11155e/ggml/src/ggml-cuda/quantize.cu#L70)
  and [PTQ matvec layout](https://github.com/PrismML-Eng/llama.cpp/blob/e3de715222b2667c54a267441159a5729f11155e/ggml/src/ggml-cuda/mmvq-ptq1_0.cuh):
  groups of32, `d=amax/127`, `roundf(x/d)`, with FP16 stored scales in the
  matvec planar layout. MMQ has its own scale/layout details.

Installed `libggml-cuda.so` strings contained both Prism environment keys. No runtime
was rebuilt or modified. The controlled reference used these three child-only
environment overrides, F32 K/V, disabled flash attention, all requested output rows,
and highest FP32 Torch matmul precision. All three public texts contain at least
ten tokens. Balanced microbatches were5+5,10+9,18+17, so both pieces stayed above
the four-token Q8 path. Original RMS<=0.0005 and maximum-token KL<=0.001 stayed fixed.

| Public case | Tokens | Native balanced-layout norm RMS | Suffix/native norm RMS | Maximum logit KL |
| --- | ---: | ---: | ---: | ---: |
| saludo |10|0.0000593808|0.0001804630|9.9296e-8|
| plazo |19|0.0003112566|2.5220e-7|5.7983e-12|
| ruido |35|1.2103e-6|5.1011e-7|1.0946e-10|

Fresh native repeats were bit-identical; all suffix cases returned `criterion=strict`.
Native plus separate Torch reference phases took38.31s under the external owned-child
guard, with measured maximum50°C and6640MiB total device use. The exact PTQ
head is `[4096,248320]`, type143. Its full F32 cuBLAS conversion alone is
4,068,474,880bytes (3.789GiB), explaining the narrow native-reference memory margin.
Longer text/native output allocations still require a separate measured check.

The second authorized experiment performed two real optimizer updates over the
35-token public text, with all248320 vocabulary classes and frozen base. Loss
5.4027290344→5.4024181366; all A/B gradients were finite, and all four were
nonzero at step2 (A zero on step1 is expected from zero-initialized B). It took
9.30s guarded elapsed; measured maximum49°C and1607MiB total device use.
Torch peak allocation was730,003,456bytes. These are mechanical updates, not social
learning evidence. No checkpoint from this throwaway backward check is a candidate.

Receipts: `strict-extraction.json`, `strict-suffix.json`, `strict-guard.json`,
`strict-backward.json`, `strict-backward-guard.json`, beside this report. They bind
the exact base hash3561df7a…28b4e8b and Prism commit
e3de715222b2667c54a267441159a5729f11155e. Extraction includes full runtime-library
hashes, shim hash, cache paths and environment. Suffix binds extraction bytes;
backward binds suffix bytes. Preserve these evidence files unchanged.

**Guard timestamp clarification:** each guard's first sample was taken before the
phase loop and its `elapsed` field contains raw `time.monotonic()` (about141 in
`strict-guard.json`). Subsequent samples subtract experiment start and are relative
seconds. Treat sample0 as precheck raw-monotonic, not as a late experiment sample;
the final `seconds` uses a consistent start/end clock. This logging defect does not
change the independent temperature/memory checks or subprocess termination rule.
The guard held the already-existing stopped worker's flock without rewriting its
contents and could terminate only the new process group it created. No power/fan
or driver changes were made.

This result establishes a controlled FP32 **prefill** reference. It does not prove
normal Q8 serving, one-token decode, nonzero candidate/native parity or deployment.
The three precision overrides changed both activation quantization and TF32; this
experiment does not isolate each one's contribution to the original error.

## Frozen code changes and CPU checks

Code frozen for root's independent review after the second experiment:

- `native.py`/`cli.py`: explicit optional `strict-f32` profile, child-only environment,
  balanced native microbatch proof, runtime hashes and profile-labelled caches.
  The legacy native path stays separately labelled diagnostic.
- `verification.py`: strict profile cannot use the old native-layout-noise escape;
  explicit highest Torch precision; backward profile receipt; all native adapter
  children exit before Torch suffix allocation in `verify_native`.
- `admission.py`/`worker.py`: before CUDA, require a strict extraction→suffix→backward
  chain and unchanged runtime hashes; training cache profile/environment/runtime
  must match. Prepare keys fresh caches by profile. Reject mixed-domain runs and
  incomplete selected LoRA A/B pairs. Run/resume identity binds config, settings,
  full example content, dataset/cache receipt bytes, init-checkpoint digest and
  relevant implementation hashes. Best/last checkpoints carry admission/settings.
- New CPU tests cover strict environments/layouts/tolerances, gate-chain rejection,
  runtime changes, data/config resume identity, complete A/B pairs and native/Torch
  allocation order. Newly added tests were observed failing before implementation.

Fresh command from `training/wanda_bonsai2`:
`PYTHONPATH=src /opt/archie/iris-venv/bin/python -m unittest discover -s tests -t .`.
Result:52 tests,51 passed and1 explicit hardware-opt-in skip; `git diff --check`
clean. Mocked phase-order coverage proves control-flow order only; the real separate
process evidence above proves the numerical experiment's actual phases.

## Small own-training protocol for root review

Root admitted only candidate SHA
`98c9929a3b8836bd25b422d499f9648933c5577084cac673f859e5ffd5ae5461`
for a small unpromoted experiment after scoped independent data review. Human review
remains pending. The old development anchor and old four runs stay intact.

1. Pin the full candidate bytes and select work/train families by sorted family ID,
   before looking at any model output: `r2-bono-metrica` and `r2-cabina-interprete`.
   Retain clean/noise/cosmetic/counterfactual variants of both, selected seed17.
   Record subset bytes/hash, the parent candidate hash and selection IDs. Run the
   native vocab-only CPU tokenizer; all full prompt+JSON targets must fit512. If
   any fails, reject that family/set rather than truncate or silently replace it.
2. Use a new owned run root,512-token config, rank8/alpha16, batch1 and fresh
   `precision_profile=strict-f32` preparation. Never reuse/relabel the legacy native
   Q8 caches. Maintain the1GiB cache and2GiB disk floors and7GiB device ceiling.
   Preparation is a potentially heavier native phase because of the F32 head; the
   short-text proof above is not its memory/thermal proof.
3. Root must authorize the next GPU phase after code review. Hold stopped-worker
   ownership lock, require temperature<60°C, continuously stop only owned children
   at>=70°C/resource/deadline breach; no old queue/control/watchdog changes.
   Complete native preparation and release native processes before any Torch phase.
4. Call `Worker.train` directly in that own root, no scheduler loop, with admitted
   dataset, strict extraction/suffix/backward receipt paths, `targets=[q,v]`, seed17,
   lr1e-4, accumulation1, epochs1, maximum optimizer steps2, eval_every1 and a bounded
   duration. A train-only slice has no quality-selection dev metrics. Save actual
   step/optimizer/RNG state, best/last checkpoint hashes and receipt. Do not claim
   checkpoint resume was exercised unless it actually was.
5. Export an unpromoted GGUF only after reviewing the actual checkpoint. Bind its
   hash to checkpoint bytes, base/runtime/tokenizer identity, profile, exact target
   names/rank/alpha, domain, parent/subset data digests and gate receipt hashes.
   The current exporter lacks some of these manifest fields; no unlabelled default
   may masquerade as a fully reviewed package. Preserve the previous packages.
6. Evaluate native free generation from the rendered **prompt only**, without any
   gold target prefix. Require a complete generated decision JSON, separately
   report invalid/truncated responses, selected option, grounding and uncertainty.
   Compare base/adapter under the same declared native settings with native outputs
   actually retained. The current JSON instruction does not justify treating the
   first next-letter distribution as the model's decision. Normal one-token native
   decode remains Q8 even with the strict-prefill environment; report that limitation.

## Remaining risks and smallest follow-ups

- Existing `Worker.evaluate` uses the full teacher-forced gold response up to
  `selected_option_id`, after gold alternatives/consequences. Its90.625% is not
  blind prompt-only decision accuracy. Preserve it as likelihood diagnostic only;
  the four unchanged-run scores do not establish social improvement.
- New admission makes old jobs intentionally ineligible in this isolated code until
  they supply the declared strict chain/profile. Do not point Claude's old worker at
  this branch or mutate its jobs to bypass that rule. Existing old checkpoints do
  not have the new binding and must not be resumed as if they did.
- Review `init_from` provenance: full A/B pairs and bytes are now bound, but existing
  code does not validate the imported checkpoint's admission/domain manifest.
  For the first two-step own run, use no init checkpoint. The initial frozen entry
  point precision omission was fixed in the scoped review round below.
- Existing malformed/missing control files default to `run`; worker lock open uses
  truncation before acquiring flock. These remain old lifecycle risks; direct own
  two-step execution under the outer read-only ownership guard avoids relying on
  those behaviors. They are not a reason to edit another owner's controls.
- `verify_native` phase overlap was fixed, but its old small-effect/noise acceptance
  remains separately labelled and is not a strict serving-parity proof. Candidate
  export/reload must be reviewed before promotion, including checkpoint/candidate
  correspondence, enable/disable restoration and nonzero adapter effect.
- For exact normal-Q8 forward matching, a future activation fake-quant path could
  reproduce native group32 rounding/scales after Hadamard. Backward would need an
  explicitly disclosed straight-through estimator: it is not the true derivative
  of rounding. MMVQ FP16 scales, MMQ scale layout, fused Hadamard/quantization and
  accumulation order require real native projection/whole-suffix receipts. Do not
  implement a guessed `torch.round` or claim canonical QLoRA. This is follow-up to
  the working controlled reference, not a prerequisite for declaring a limited,
  correctly labelled FP32-prefill experiment.

## Scoped independent code review and CPU repair round

Root's independent reviewer found three real admission defects after the first
freeze: `Worker.train` did not explicitly pin the same highest FP32 policy as the
reference; minimal extraction/backward fixtures could be admitted without actual
measurements; and NaN native comparison metrics bypassed `>` checks. Root authorized
one narrow repair round. Tests reproduced these failures before the changes.

The second frozen version pins highest Torch matmul and `allow_tf32=False` before
worker allocations, rejects conflicting Torch/NVIDIA TF32 overrides, and records
the effective process policy inside the already-hashed admission record carried
to checkpoints/receipt. Admission now validates three complete finite native
extraction cases, suffix case/token correspondence, highest precision and two
finite full-vocabulary backward steps with the complete q/v gradient names and
strictly positive A/B norms at step2. It explicitly admits q/v only until another
target set has its own measured proof. Native RMS/KL/distribution metrics reject
NaN/Inf, negatives and booleans; fresh training caches must also match the strict
reference shim hash.

Fresh full suite:57 tests,56 passed and1 explicit hardware-opt-in skip; diff check
clean. The strengthened admission function was also run CPU-only against the
unchanged actual three strict receipts and installed runtime-library bytes: passed.
No numerical operator changed in this repair round; no additional GPU execution
was requested or performed. Root requested a scoped re-review before semantic work.

CPU native vocab-only checks rejected both initially preselected families at512:
`r2-bono-metrica` clean586/noise645/cosmetic586/counterfactual576;
`r2-cabina-interprete` clean519/noise571/cosmetic520/counterfactual516.
Root then explicitly authorized length-only eligibility across all eight work
families, taking the first two sorted families whose complete four variants fit512,
or one if only one qualifies. No model outputs or dev scores participate. Preserve
that length ledger and selected IDs with the source hash; do not raise the limit,
truncate, or silently edit the approved candidate if none fit.
