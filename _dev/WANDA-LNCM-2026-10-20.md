# Wanda LNCM — implementation log (2026-10-20)

Owner request: implement Wanda as an LNCM (low-rank adapters over a next-concept model)
that refines a language model's answer, reads long social context in chunks, keeps a
persistent per-person state ("what to expect of someone"), answers in two states, and can be
trained by distilling several open teachers on Nebius — complete enough to continue training
without revisiting the base code.

## What was added

- `core/crates/archie-wanda-lncm` (new): codec (bytes + Rust BPE), multiscale residual
  VQ-VAE, prefix-LM NCM generator with gate/kind/change/decision heads, LoRA per domain,
  checkpoint manifest with SHA-256 verification, social state + delta grammar, chunked
  reader, synthetic worlds, dataset schemas and tranche import, distillation pipeline,
  training stages, crisis rule, CLI `archie-wanda-lncm`.
- `core/crates/archie-wanda`: optional `[lncm]` config; `social.rs` (Vault persistence per
  domain, incremental reading behind a cursor, states 1/2, people view and owner edits);
  `QueryResponse.quick/refined`; `/v1/people`, `/v1/people/edit`; crisis state.
- `core/crates/archie-voice` (`wanda` feature): the crisis message is spoken without the
  "Propuesta de Wanda" framing.
- Docs: `docs/WANDA-LNCM.md`, `training/wanda_lncm/README.md`,
  `training/wanda_lncm/distill.example.toml`, HACKATHON.md section.

Authority boundaries are unchanged: the expert proposes; Cerebra, Policy, leases and the Stage
keep every decision. Personal and work domains keep separate state entries and adapters.

## Verification (this session, cloud container, CPU only)

- `cargo test -p archie-wanda-lncm`: unit tests plus `tests/pipeline.rs` (tiny end-to-end
  training/serving/tamper refusal) and `tests/distill.rs` (teachers, drafts, judges, selection
  against loopback OpenAI-compatible stand-ins, resumption, private-shape refusal).
- `cargo test -p archie-wanda`: existing suites unchanged and passing, plus `tests/lncm.rs`
  (events → Vault state → states 1/2 → people view → forget → crisis → tamper refusal).
- Rust 1.88 (the CI toolchain): `cargo clippy --workspace --all-targets --locked -- -D warnings`
  clean; `cargo fmt --all -- --check` clean.

Not verified: CUDA builds, GPU latency, any trained model's quality or any benchmark. The
tiny models in tests are trained for a few steps and are meaningless as models.

## Next (owner/operator)

1. `archie-wanda-lncm synth` at scale and `import-tranche` for the `tomgen` tranches.
2. Distill on Token Factory with `distill.example.toml` (set `eval_only` to the FANToM and
   OpenToM files first).
3. Train `small` on the RTX 3070 (thermal guard on) or `base` on Nebius AI Cloud; calibrate;
   `eval`; only then point Wanda's `[lncm]` at the checkpoint.
