# Core daemon integration — 2026-10-21 22:50 UTC

Repository verified before edits: `origin=https://github.com/PentaVM/ArchieOS-Nvidia.git`, isolated `codex/router-memory-harness` worktree. No commits, pushes, excluded branches, GUI/WSL generation, training or GPU execution.

## Implemented contract

- Moved the existing pure `ConstraintSource`, `WorkerResumeContext`, `CollaborationContext`, `resume_context` and `collaboration_context` into `archie-memory::resume`. Core reexports every existing symbol at its original path. A normalized comparison against HEAD confirmed the implementation is unchanged beyond namespaces/whitespace. Workstation imports memory directly and drops its Core dependency, allowing Core to consume the unified service without a dependency cycle.
- `ControlRequest::AiInfer { request: InferenceRequest }` has JSON tag `request="ai_infer"` and nested field `request_body`. The field rename avoids colliding with the existing internal enum tag. `AiRecall { binding: InferenceBinding, receipt_id: String }` recalls one exact committed binding. `ControlResponse::AiResult { outcome: Box<InferenceOutcome> }` returns the attributed result. Boxing does not change JSON.
- Both requests are marked mutating/bound disclosure (`is_ai()` and `is_mutating()`). `Runtime::handle_control` rejects them before opening inference configuration. No executable, process arguments, working directory, policy path, credentials or caller-supplied authority are accepted on the wire.
- The actual daemon already uses `serve_operator_connection`. Its narrow AI branch captures Linux kernel peer UID/PID before splitting the connection, separately from the unchanged Sentry foreground-terminal path. Only the crate-private peer can enter the trusted AI dispatcher. Windows/development loopback cannot create this peer and is refused. The service reloads the private profile and admits exactly its kernel-identified owner on every request and before returning content.
- The lazy dispatcher clones `Runtime.state`'s existing `Arc<StateStore>`; it never opens another writer/database. It snapshots the current Runtime Policy engine per dispatch, loads the fresh shared provider/model catalog, and uses the common policy/Vault/inference service. Profile/domain/runner checks remain outside the model.
- The existing one-MiB control framing bound remains. The AI operation has a profile-derived deadline plus bounded persistence grace. EOF or pipelined input sends cancellation to the service, gives it two seconds to persist, and then drops the active owned transport if needed.
- `StatePaths::inference_harnesses()` uses canonical state `inference/harnesses.toml`, with the existing workstation catalog as a compatibility fallback. Configured Codex commands come only from a bounded regular operator/root-owned file that is not writable by others. Duplicate Codex entries, custom arguments and endpoints are refused. With no catalog, the fixed official `codex` PATH executable is used. The managed-client working directory must be private.

## Verification actually run

All Cargo commands used the single coordinated slot, `CUDA_VISIBLE_DEVICES=`, `CARGO_BUILD_JOBS=1`, existing `/var/tmp/archie-pr-integration-target`, and debug info disabled. No alternate build cache was created.

1. Compile-RED before implementation: memory `--test resume` failed because `archie_memory::resume` did not exist; Core `--test ai_control` failed because `AiRecall`/`is_ai` did not exist.
2. `cargo test --offline -p archie-core --test ai_control -p archie-memory --test resume`: exit 0; three Core wire tests and two pure memory-resume tests passed. Two database tests were explicitly ignored in that invocation.
3. `cargo test --locked --offline -p archie-core -p archie-memory`: exit 0. All nonignored suites passed, including 127 Core unit tests, 84 memory unit tests and the remaining protocol/integration/doc suites. Existing environment-gated tests remained ignored; this was not a claim that all ignored fixtures ran.
4. With the root-owned disposable PostgreSQL 18 cluster on loopback port 55439 (`archie`, `postgres`, TimescaleDB 2.29.2/vector 0.8.6): `cargo test --locked --offline -p archie-core --test ai_control -- --ignored --test-threads=1`: **2/2 passed**. This was rerun after extending the owner test with disconnect cancellation.
   - Context-free dispatch refused inference before creating inference state/configuration.
   - A real Unix peer owned the profile, with no Sentry UID configuration. A provider installed after boot was discovered. Captured HTTP inference completed through the existing Runtime store and filesystem Vault, and the resulting receipt was visible in that same `Runtime.state`. Exact bound recall returned the same result. Context-free recall was refused. Changing allowed domains on the same established connection refused recall.
   - A second actual HTTP request deliberately withheld its response. Closing the operator socket terminated the connection within the bound. `recent_session_inferences` contained a durable `Cancelled` receipt, and exactly one `Completed` receipt remained (the prior completed turn).
5. `rustfmt --edition 2021 --config skip_children=true --check` over all owned Rust files/helpers: exit 0. `git diff --check` on the owned paths: exit 0.
6. Strict Clippy session 54306 was resumed to completion rather than duplicated: exit 1 at the shared dependency `archie-workstation/src/codex.rs:170`, unused `mut cancel` under `-D warnings`. It did not reach a final Core lint verdict. The unified-service fix agent owns that correction; the Cargo slot was released to it. Strict aggregate Clippy still requires a successful rerun.

The HTTP fixture proves execution, persistence, cancellation and scope integration. It does not verify model quality, subscription entitlement, hardware, real-provider latency or deployment. Native Windows daemon AI denial is implemented by the absent kernel peer; a native Windows end-to-end daemon test was not run here.

## Ownership handoff

Core source/manifests/tests, memory resume source/export/tests, workstation dependency removal and the StatePaths harness helper are ready for root integration. Workstation `ai.rs`/`ai_policy.rs` contain only this agent's agreed import-line replacements amid root-owned untracked implementations; root should stage those complete files after its service fixes. No independent service/CLI changes were made. Root owns the ongoing service lease/budget/Vault concurrency corrections and daemon-preferring CLI. Those later changes require aggregate validation; the test evidence above applies to the integration snapshot actually exercised.

## Harness² daemon extension — in progress

Added `AiHarness { request: HarnessRunRequest }` (JSON `request="ai_harness"`, `request_body`) and boxed `AiHarnessResult`. It uses the same private kernel peer, exact profile owner, fresh catalog, existing Runtime StateStore, cancellation watch and disclosure checks. Worker paths come only from `state_dir/inference/harness2.toml` through root's trusted `load_program`; the state root is fixed by Core. The whole-run duration is 100–300000ms with three seconds of bounded cleanup grace, while each stage retains its independent profile limit. Root implements the shared stage adapter and total-deadline cancellation/drain. Core now checks fresh domain/runner scope again before returning any AI content.

Witnessed behavior-RED: `ai_control`'s new Harness wire test failed on unknown variant `ai_harness`. After implementation, all four CPU wire tests passed and both actual PostgreSQL/Unix inference/recall tests passed with the new shared API. Recall checks receipt/text equality and absence of invented historical budget observations.

New `harness_control` tests launch the actual pinned Python worker from a private disposable Linux copy of worker/manifest/vendor source. DrvFS checkout files are mode777; launch checks correctly refuse those source files directly, so the fixture copies exact bytes into owner-only files rather than weakening launch policy.

- The 250ms whole-run / 60s per-stage stalled-HTTP regression **passed**: actual HTTP dispatch began, Core returned an error, and assertions through `Runtime.state` found a durable Cancelled receipt and zero Completed receipts.
- The initial eight-stage success test failed after exactly one Completed baseline receipt. The discovered fixture catalog deliberately has a conservative 4096-token context budget, which does not fit the actual upstream proposer prompt. The fixture now explicitly installs a 32768-context descriptor for its controlled synthetic HTTP server; production discovered budgets remain unchanged. This adjustment is not yet verified.
- The retry was interrupted by the ongoing monetary Option migration: `harness_runtime.rs` assigned `Option<u64>` to the still-`u64` `Harness2Limits.max_cost_micros`/`per_call_cost_micros` fields. Those shared files are owned by root/review agent. No Core compile error was reported. Cargo was released; wait for that coordinated migration before retrying the positive worker test and strict aggregate lint.

The Harness follow-up source/tests are not yet staged as complete. Previous passing Core evidence is preserved above; do not describe the new eight-stage daemon path as verified until its success test passes.
