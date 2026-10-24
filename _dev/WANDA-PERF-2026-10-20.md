# Wanda runtime: memory, caching, parallelism, context — audit and first fixes (2026-10-20)

Owner request: find where Wanda spends memory traffic, where to cache, how to run prompts in
parallel, how far the context can grow, and whether kernel-level work (a megakernel, as in
Hazy Research's "Look Ma, No Bubbles!") would make it faster on ArchieOS.

## Where the time goes today (measured on the RTX 3070 PC, WSL Operator)

Wanda's answer is two models in a row: the local LLM (Cerebra's reasoning worker, served by
llama-server on 127.0.0.1:8081: Ternary-Bonsai-2-9B, PTQ1_0, 2.0 GB) and then the LNCM.

| Path | Before | Finding |
|---|---|---|
| LLM, 1 986-token prompt | 2.8 tok/s read, 2.1 tok/s write: **739 s** | CPU only (`--n-gpu-layers 0`, GPU hidden), 2 of 16 threads, `--ctx-size 4096`, one slot |
| Same, 12 threads, q8 KV, flash attention | 5.0 / 4.6 tok/s: 405 s | CPU-bound; two parallel slots only split the cores (3.9 tok/s each) |
| Query budget | 20 s per model call | Unreachable on CPU for any real prompt |

The LLM, not the LNCM, decides Wanda's latency. On the GPU it is a ~2 GB model.

## The model's context

Bonsai-2-9B is a Qwen3.5 hybrid: 32 blocks, full attention every 4th (8 layers keep a KV
cache, 4 KV heads × 256), the rest Gated-DeltaNet with a fixed recurrent state. It was trained
to **262 144 tokens**; it was served at 4 096. KV cost: 32 KiB/token in f16, ~17 KiB in q8_0.

| Context | KV f16 | KV q8_0 | Fits |
|---|---|---|---|
| 32 768 | 1.0 GiB | 0.55 GiB | CPU RAM (WSL has 7.7 GiB) or GPU |
| 131 072 | 4.0 GiB | 2.2 GiB | GPU with q8 (weights 2 GiB + buffers) |
| 262 144 | 8.0 GiB | 4.4 GiB | GPU only with q8 and nothing else on it |

`wanda-llm.sh` (operator script, outside the repo) serves `cpu` (32k, 12 threads) while the
LNCM trains and `gpu` (128k, 4 slots, continuous batching, `--cache-reuse 256`) afterwards.

## LNCM: memory calls and fixes in this change

| Where | Was | Now |
|---|---|---|
| `nn::RmsNorm` | 5 ops, 4 temporaries per call | fused `rms_norm` when no gradient is tracked |
| `nn::Attention::attend` | `K.t().contiguous()` copy every layer and call; generic softmax (3 kernels) | transposed read, no copy; fused `softmax_last_dim` in use |
| `Lncm::quick` | re-ran the gate prompt the service had just run | `quick_after_gate` reuses it (one encode fewer per question) |
| `Lncm::compose`, N candidates | N × (levels) passes at batch 1 | `generate_many`: one pass per level for all candidates, prompt KV shared |
| `social::answer` | opened and parsed the whole Vault (every stored message) per question | state read from memory; only `save` writes it and refreshes the copy |
| `runtime::query` | read **all** unread messages through the LNCM before answering | reads at most 48 turns inline; the proactive tick reads 480 per domain |
| `server::query` | one in flight, any other got 503 at once; the reader held the same permit for a whole backlog | waits up to 25 s for a permit; `parallel_queries` (1..8) matches llama-server slots |

Kept for later, measured first: the prefix KV is concatenated with each level's keys
(`Tensor::cat`) per layer and level — a preallocated KV buffer removes it; masks for training
batches are built on the host per step — cache them per width.

## Kernels: megakernel, honestly

The LNCM small preset is ~22 M parameters (d 384, 6 layers). At batch 1 its forward is
launch- and allocation-bound, which is exactly the regime a megakernel targets: one persistent
kernel per pass instead of ~25 launches per layer. Steps, in order of risk:

1. Fused ops already in candle (done above) and batching (done above).
2. CUDA graphs for the fixed shapes (prompt pass at padded widths, one graph per level):
   removes launch overhead without new kernels; needs candle's allocations to be capturable.
3. `candle-flash-attn` (sm86 supported) behind a feature, for the prompt pass and training at
   longer prompts.
4. A persistent fused kernel per transformer block (RMSNorm → QKV → attention over the cached
   prefix → O → MLP), written against the exact small-preset shapes, then the whole level pass.
   Only after (2) shows the launch share that remains; a hand-written kernel must be measured
   against cuBLAS on this card, not assumed faster.

For the LLM, llama.cpp already fuses most of what matters (flash attention, quantised matmul,
graph reuse); its win is the GPU and the context, not a new kernel.

## LNCM context

`prompt_len` is 768 tokens with learned absolute positions (`gen.pos`), so it cannot be raised
at serving time. The reader already makes the conversation length unbounded (chunks + a
persistent state); what grows the per-question context is a v2 generator: rotary positions,
`prompt_len` 2048–4096, memory-efficient attention for training, and longer synthetic worlds.
That needs a training run of its own; the current run (small preset) is not changed.

## Verification

- `cargo test -p archie-wanda-lncm` and `-p archie-wanda` (CPU), new tests:
  `batched_candidates_match_one_at_a_time`, `a_bounded_read_takes_the_earliest_and_loses_nothing`.
- `tests/bench.rs` (ignored): latency and host allocations per entry point at the real
  checkpoint's shape. CPU (WSL, 16 threads, shared with a GPU training run), fixed
  pseudo-random weights, median of two interleaved rounds of 15:

| Entry point | Before | After |
|---|---|---|
| gate | 26.8 ms | 26.4 ms |
| quick (gate + decide) | 211 ms | 165 ms |
| compose, 1 candidate | 504 ms | 393 ms |
| compose, 4 candidates | 1 559 ms | 1 185 ms |
| update | 883 ms | 632 ms |
| gate + quick + compose | 716 ms | 596 ms (host copies 796 → 585 MiB) |

  Fused ops on a CPU only pay for large tensors (gate prompt: 24 ms composed, 33 ms fused),
  hence `fused_pays`. The service also skips the second gate, which the bench still runs.

Not verified: any GPU latency of the LNCM (the GPU was training), CUDA graphs, flash attention,
a megakernel, or the 128k GPU serving (it starts only when the training run ends).
