# Unified service / managed Codex independent review

Reviewed 2026-10-21 18:07 UTC, branch `codex/router-memory-harness`; `origin` verified as `https://github.com/PentaVM/ArchieOS-Nvidia.git`. Scope: workstation `ai.rs`, `ai_policy.rs`, `codex.rs`, `harness.rs`, `tests/unified_runtime.rs`, `tests/codex_managed.rs`; memory, PostgreSQL, Vault and Policy code read only to establish their binding contracts. Unfinished Core/CLI/Harness handlers and the separate provider review are excluded.

This report records the pre-fix source examined during review. Root began fixes after the findings were sent; those fixes are **not reviewed or verified here**. Original SHA-256: `ai.rs` `7fcdf3a92e59e15d58d8b0bf66c65fe66694f587fb9fe63ba5e456ef866cc5e4`, `ai_policy.rs` `1fb08736f7ba687cbf0e9d1f74f90c918eb3ebd69d833688ef03f3b257335f47`, `codex.rs` `ebec0d08666cb79586436cf72562318e4f9aaa0a9fafd1991d5b135882d370a1`.

## Important findings

### 1. P1 — First-turn account switch crosses the admitted account boundary

Location: `core/crates/archie-workstation/src/ai.rs:196-208`, `230-245`; `src/codex.rs:385-405`.

The service obtains account A through `client.account()` in one child, binds the task to A and recalls A's evidence. `complete()` launches a second child and obtains a fresh account B, but compares B only against an existing `CodexSession`. For a fresh task without a prior session there is no expected-account check. The service then ignores `answer.session.account_id` and commits the returned text/thread under A's binding. Switching the official client account between these calls therefore sends A-scoped context under B and misattributes the result to A. Public-only subscription policy does not remove the exact account/domain contract.

Reproduction: a managed protocol fixture returns A for the service's account child and B for the completion child; make a first inference with no stored session. Before the fix the second child receives `thread/start` and `turn/start`, and its result is prepared for an A receipt.

Fix: pass the account pinned by service admission into the completion operation; compare the freshly observed account **before `thread/start`/`thread/resume` or transmission of the capsule**. Check the returned account again before committing. Regression must assert no thread/turn RPC occurs on mismatch, including the no-prior-session case.

### 2. P1 — Resuming the official mutable thread restores uncommitted output

Location: `core/crates/archie-workstation/src/ai.rs:218-236`, `247-277`, `638-691`; `src/codex.rs:399-417`.

After a successful committed turn on thread S, later requests reuse S. A cancelled/failed turn, an answer rejected by a changed policy, or a completed turn whose checkpoint CAS loses still mutates the official thread. Failed receipts have no new session ID and a rejected commit has no receipt, but `inference_session()` can return S from the earlier successful receipt. The next `thread/resume` restores the later rejected history from Codex even though Core's capsule and active checkpoint lineage exclude it. `CodexCompletion.turn_id` is discarded, so there is no committed remote-history anchor or rollback before reuse.

Reproduction: success on S → append a synthetic marker in a second turn which fails/cancels or loses CAS → issue a third request. The client resumes S, whose history contains the rejected marker despite its absence from admitted committed evidence. A successful SQL receipt alone cannot make a mutable external thread a snapshot.

Fix within this slice: start a fresh service-managed Codex thread for every admitted attempt and reconstruct context solely from committed Core/Vault receipts. Keep opaque returned thread IDs as attribution. If low-level owner-facing adapter resume remains available, distinguish it from service memory recovery. Regression should use a stateful protocol fixture and prove that the service never resumes the contaminated thread after a failed/cancelled/CAS-rejected attempt.

### 3. P1 — Concurrent full-snapshot Vault writes can erase committed results

Location: `core/crates/archie-workstation/src/ai.rs:602-630`; supporting existing helper `core/crates/archie-cerebra/src/vault_io.rs:122-140`.

`finish()` opens the shared Vault snapshot, adds a result, rewrites the complete `entries.json`, then reopens and verifies it before committing PostgreSQL. There is no cross-instance/process lock over that sequence. Two runtimes on distinct Focus IDs can read snapshot V, A write/verify/commit V+A, then B write/verify/commit its older V+B. Both PostgreSQL CAS operations succeed because their Focus IDs differ, but A's committed artifact disappears. The helper's shared `.entries.json.partial` filename also allows overlapping writers to interfere before rename. `&mut UnifiedRuntime` serializes only one runtime object.

Reproduction: two runtime/storage instances, one Vault directory, distinct Focus IDs, barrier after each opens V. Release A through verification/commit, then release B. A's receipt remains committed while its Vault result can no longer be recalled.

Fix: hold a cross-process lock for the shared Vault directory around the entire open → store → persist → reopen/verify sequence (including key initialization where relevant). Unique temporary filenames alone do not fix lost updates. Regression must reopen after two concurrent distinct-Focus writes and recover both committed outputs.

### 4. P1 — The actual Core lease is discarded before the execution boundary

Location: `core/crates/archie-workstation/src/ai_policy.rs:183-206`; `src/ai.rs:65-68`, `181-182`, `449-455`, `584-589`.

Admission checks and consumes the granted lease, then drops it and returns descriptive constraints. `Prepared` keeps only the execution head and profile-file hash. Vault/context preparation and process/network setup occur afterward, with no retained lease expiry or check immediately before the external operation. The Policy engine may clamp a requested duration to `max_lease_lifetime_ms`, but the runner continues using `profile.max_duration_ms`. At completion `admit()` creates a new lease, which cannot establish that dispatch occurred within the original grant.

Reproduction: configure a short Policy lifetime, a longer profile duration, and a barrier/delay between `prepare()` admission and provider dispatch. Advance beyond the granted expiry, release the barrier: dispatch still proceeds. With a slow provider the current finish path can also publish success using a newly minted grant. Whether an already-started text operation must stop at expiry is a separate policy choice; dispatch after expiry is outside the original authority either way.

Fix: retain the actual grant's identity/expiry in prepared state; check it again at dispatch and derive the attempt deadline from the admitted bounds. Preserve the fresh completion policy check as an additional check, rather than allowing it to renew the original operation implicitly. Regression should use a Policy lifetime shorter than the profile limit and prove no external dispatch after original expiry.

### 5. P2 — Opaque message-ID sorting changes the answer and persists commentary

Location: `core/crates/archie-workstation/src/codex.rs:425`, `450`, `470-482`.

Completed `agentMessage` items are accumulated in a `BTreeMap` keyed by opaque item ID; `into_values()` produces lexical ID order instead of protocol event order. The adapter also joins commentary and partial-answer items into the final answer even when the protocol explicitly identifies a `final_answer` item. This can persist progress narration or reordered answer fragments as the completed model proposal.

Reproduction: emit completed items with IDs `z` then `a`, then a completed turn. Text is returned in `a,z` order. Emit a `commentary` item followed by `final_answer`; both are currently persisted as the answer. The installed official JSON schema explicitly supports `commentary`, `partial_answer`, and `final_answer`, with nullable/unknown phase for legacy compatibility.

Fix: preserve observed item order while deduplicating by ID, and select terminal `final_answer` text when available. Keep an explicit ordered fallback for models whose phase is absent. Regression should cover nonlexical IDs, duplicate item updates, commentary plus final answer, and legacy phase-null events.

### 6. P2 — Managed inference silently ignores the admitted output-token limit

Location: `core/crates/archie-workstation/src/ai.rs:169-170`, `230-237`; `src/codex.rs:161-169`, `417`.

`InferenceRequest.max_output_tokens` is validated against the operator profile and passed to API runners, but the managed completion signature has no output budget parameter and never applies the limit. A request for one token can consume/return thousands under the independent 64 KiB text and total-time limits. Therefore the same admitted service request has materially different budget semantics depending on runner.

Reproduction: request `max_output_tokens=1` through a managed fixture that emits a larger completed answer; it succeeds. The installed stable `TurnStartParams` schema has no direct max-output-token field, so adding an invented JSON field would not fix this.

Fix: represent/enforce the managed runner's supported budget explicitly. Use a supported official limit if one exists for the installed version, or fail closed for an unsupported requested hard token bound and expose the actual enforced byte/time limits. A post-hoc text-size check must not be described as a provider generation-token cap. Regression should prove a requested cap is either enforced by a supported mechanism or rejected before inference.

## Quality / specification verdict

The implementation has substantial concrete protections: exact-domain admission before Vault decryption, persisted runner/provider/account binding, observed terminal-status checks, bounded JSONL frames/cumulative child output, owned-child kill-on-drop, total process timeout, public-only managed subscriptions/UHP proxies, recall-grade checks, no verified-fact promotion, and PostgreSQL checkpoint CAS. No replacement architecture is needed to fix the findings.

The reviewed snapshot is **not ready to claim complete durable scoped inference** until findings 1–4 are fixed and proved. Findings 5–6 should be addressed before presenting the managed runner as equivalent to the admitted text-inference contract. The single restart integration test covers a successful serial path and domain denial; it does not cover these account-rotation, contaminated remote history, concurrent shared-Vault, lease-expiry, phase/order, or managed-budget cases.

Review evidence: source inspection only; no Rust test suite, model turn, GPU work, token/config-secret read, or database mutation performed by this reviewer. Root supplied the earlier evidence of 25 workstation unit passes, one real PG18/Timescale/vector + filesystem Vault + captured HTTP restart integration pass, and a successful official Windows Python public synthetic turn. Those are acknowledged as existing evidence, not independently rerun or broadened here. `tests/codex_managed.rs` existed but its actual Rust managed-turn run was still pending at review dispatch.

Official protocol validation: the installed Windows `codex.exe app-server generate-json-schema` succeeded into `C:\Users\Penta\AppData\Local\Temp\archie-protocol-review-e367afb045c64f16854f803740807043`; only generated protocol schemas were inspected. The lifecycle and schema-generation behavior are documented in [official Codex App Server documentation](https://learn.chatgpt.com/docs/app-server). This confirms protocol shape, not credentials, entitlement, model quality, hardware, or a successful Rust integration turn.
