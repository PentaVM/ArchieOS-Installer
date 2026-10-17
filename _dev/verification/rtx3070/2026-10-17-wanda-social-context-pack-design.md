# Social context pack for Wanda — design note (owner direction 2026-10-17)

Owner direction: the cognitive layer (Cortex) should hand Wanda a compacted context carrying
the social signals: parsed text, intentions, summaries of WhatsApp, email and similar
sources, around 24k to 50k+ tokens. Real conversations are long, lab questions are short, and
the KV cache should be reused.

## Measured constraints (RTX 3070, Bonsai2 9B PTQ1_0, Prism b10777)

- Hybrid model: 8 of 32 blocks are full attention with KV (4 KV heads × 256, f16); 24 are
  Gated DeltaNet with a fixed-size recurrent state. KV ≈ 32 KiB/token: 24k ≈ 0.75 GiB,
  50k ≈ 1.6 GiB, 128k ≈ 4 GiB, next to the 2 GiB of weights.
- Prefill ≈ 1,650 tok/s and decode ≈ 80–90 tok/s, measured in `llama-server` logs on
  ~300–430-token prompts. Cold prefill is therefore ≈ 15 s at 24k and ≈ 35–45 s at 50k.
- Prefix reuse with recurrent state works only for an identical prefix up to a saved
  checkpoint (`--ctx-checkpoints`, host `--cache-ram`), so the pack must be **append-only**.
- Training: the Torch suffix uses full attention (50k² scores would need ~160 GB) and a
  50k-token prefix cache is ~800 MB per example. Long context is therefore evaluated, not
  trained; training uses compacted problems of ≤4–8k once chunked attention exists.

## Pack layout (order chosen for cache reuse)

1. Stable header: people, relationships, stated preferences, active domain (personal and
   work never mixed).
2. Knowledge ledger, append-only: each fact or change → who was present or received it,
   channel, time, Vault evidence reference. This preserves the information asymmetry that
   generic summaries erase and that ToM questions depend on.
3. Open threads: commitments, pending items, signals labelled *inferred*, never as facts.
4. Recent window verbatim; the question last.

The pack is unprivileged context under the existing Core/Cortex contracts (Task 9 decision
channel). It confers no authority, and raw chats are never admitted to training automatically.

## Measurement before any contract change

- Extend `tomgen` with long multi-thread chats (24k/50k tokens: joins, leaves and the
  decisive fact buried among distractors). Each comes in two forms, raw versus compacted
  pack, with identical questions. Report knowledge/belief accuracy by length and form.
- FANToM official `full` input as an external long-conversation check.
- Server: `-c 65536`, then measure VRAM, prefill time and cached-turn latency;
  `--slot-save-path` for per-conversation persistence.
