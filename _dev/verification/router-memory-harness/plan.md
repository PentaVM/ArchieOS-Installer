# Unified AI implementation plan

> Agentic execution: root integration plus focused parallel implementers; TDD and a fresh final review. Spec: design.md in this directory. The owner explicitly authorized autonomous execution and selected the deadline; do not stall for repeated routine design approval.

Goal: configured local/API/official-client models actually execute through one policy-aware route and persist scoped, restartable experience, with genuine Google Harness² orchestration using that boundary.

Tech stack: existing Rust workspace, Tokio/reqwest, PostgreSQL18 and Vault, official Codex app-server, pinned Python Harness². One Cargo job, existing build target, no GPU. Source baseline bca8473.

## Tasks and gates

- [x] Audit actual code, open PR exclusions and primary provider/upstream documentation. Persist scope and continuation.
- [x] Baseline: `cargo test --locked -p archie-router -p archie-model-runtime -p archie-memory -p archie-workstation`,229passed3ignored,22 suites,exit0.
- [ ] Catalog/transports (provider agent): tests first for provider config becoming routable, native Anthropic JSON/SSE, locality spoofing, bounded reads and total deadlines. Implement in model-runtime, keep workstation provider compatibility. Expose `Routed::from_paths` and announce public signatures; root adds live Core/CLI callers. Commit scoped deliverable after its focused tests.
- [ ] Domain persistence (memory agent): tests first for exact domain/account binding, receipt/checkpoint CAS and rollback/reopen; add typed memory contract, additive PostgreSQL migration and StateStore methods. Expose signatures before root consumes them. Run real disposable-PostgreSQL tests through coordinated fixture. Commit scoped deliverable.
- [ ] Google Harness² (harness agent): pin/import actual upstream, implement safe custom proposer/substrate/BenchmarkAdapter, worker/bridge protocol, global budget/resume and domain namespaces. Prove k1 PPC R4/P3 and second-task library reuse, negative path/policy/script/domain cases. Do not claim model improvement. Commit scoped deliverable.
- [ ] Root official-client execution: bounded managed Codex app-server, account status/login references, validated event lifecycle, cancellation and account/domain-scoped resume. RED deterministic process fixtures; GREEN process adapter. Check real installed CLI/account safely; no token reads.
- [ ] Root end-to-end service/CLI/Core consumers: configuration -> fresh constraints/capsule -> exact route -> runner -> observed receipt -> sealed result/checkpoint -> recall. Add real restart/in-memory negative tests and CLI smoke. Keep current public interfaces compatible. No excluded source/PR changes.
- [ ] Authenticated integration proof: bounded public synthetic turns through available real local/OpenAI/Anthropic interfaces, memory reopen and harness tasks. Record actual access versus missing credentials/approval; do not substitute version probes.
- [ ] Review/verification: independent fresh branch review; fix important findings with failing regressions; fmt, strict Clippy, affected+workspace tests, source/provenance/license checks. Commit/push work to own branch; preserve all existing PRs.
- [ ] Delivery: executable operator commands, exact receipts/limits and useful continuation. Complete the objective when no required work remains, or honestly record remaining external blockers at deadline. Remove the24h heartbeat at completion/deadline.

## Review focus

- A local proxy must not launder cloud data locality or provider retention.
- Caller-controlled domain/session/runner IDs must not bypass Core or Vault.
- Failure/cancellation or a crash between artifact and commit must not create successful recalled experience.
- Retry/reflection/editor/selection work must consume the same total deadline and budget.
- Upstream-generated code/config must not change Policy, credentials, leases, manifests or host files.

## Ledger

Audit reports completed without edits/builds/models/credential reads. First agents hit account quota before finishing; resumed existing agents after owner retry and collected reports. No claim that a model setting changed. Automation created successfully with destination=thread after initial schema error. Baseline session99094 completed exit0; detailed count pending extraction. No product implementation started before this design/plan.
