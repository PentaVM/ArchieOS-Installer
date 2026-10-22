# Scoped inference persistence

Source: `codex/router-memory-harness`, based on `bca8473`, origin verified as
`PentaVM/ArchieOS-Nvidia`. Ownership: `archie-memory`, `archie-postgres`, and the
StateStore inference adapter. No IRIS/Wanda/Sentry source, other PR branch,
GUI/WSL generation, model/GPU/training or credential storage was changed.

## Public contract

`archie_memory::inference` adds `InferenceBinding`, `ScopedReference`,
`OpaqueSessionId`, `InferenceStatus`, `ObservedUsage`, `InferenceReceipt`, and
`sha256(&[u8])`. Public `validate` methods and `receipt.validate_checkpoint` reject
malformed identities, cross-domain/account refs, unsuccessful status carrying successful output and
model-result promotion to verified facts. Usage is optional observation, never a
fabricated estimate. A failed/cancelled receipt has no successful result digest or
result ref; its parent checkpoint remains Current.

`archie_memory::storage::InferenceStore` and matching StateStore inherent methods:

- `bind_inference(&binding)` before I/O; immutable full task identity, one exact
  ContextDomain per Focus. Different provider/account tasks can share that domain.
- `commit_inference(&receipt, &checkpoint, expected_head)` atomically appends the
  attributed receipt, CAS checkpoint and both outbox entries.
- `inference_receipt(&binding, id)` reads exact-task immutable history.
- `recent_inferences(&binding, limit)` reads exact-task active-lineage receipts.
- `recent_session_inferences(&binding, limit)` reads active-lineage receipts
  across tasks in the same domain/focus/objective/runner/provider/account;
  limit 1..=128 and full registered task binding required.
- `inference_session(&binding)` reuses only a committed opaque session in that
  same scope. Another account/domain/Focus/objective cannot inherit it.

Completed receipt checkpoints use kind Result; failed/cancelled attempts use kind
Attempt. Both remain Current. `evidence_ref = receipt.provenance_ref()` links the
checkpoint to PostgreSQL attribution. Every retained state reference must be one
of the receipt's admitted scoped Vault refs or its distinct sealed result ref;
`verified_fact_refs` is deliberately refused at this boundary. Core/Vault still
own authorization, artifact existence/digest verification and disclosure.

## Migration and transactional behavior

`archie-postgres/src/inference-v2.sql` is an additive hash-checked migration. The
original `schema.sql` and its v1 hash are unchanged. A missing v2 upgrades in the
same migration transaction; unknown versions or either modified hash fail closed.
The migration adds Focus-domain bindings, immutable per-task bindings and indexed
receipt/session-scope metadata. Existing database writer locking, append-only
checkpoints, CAS, repeatable-read lineage, digest decoding and outbox are reused.

Receipt/checkpoint/outbox failures roll back together. A staged Vault artifact
without a committed receipt does not appear in store recall. Explicit historical
receipt lookup keeps attribution, while default recall/session resume filters out
abandoned checkpoint siblings after rollback. Filtering is performed in SQL before
LIMIT, so a small context budget cannot hide the older active root behind abandoned
history. External session identity is provider + opaque ID; binding it to another
account, runner, domain, Focus or objective is rejected before commit. Policy/lease authority is not saved
in the receipt or reconstructed from model context.

## Verification ledger

All Cargo commands use the operator WSL, `CUDA_VISIBLE_DEVICES=`,
`CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0`, `CARGO_BUILD_JOBS=1`,
and `CARGO_TARGET_DIR=/var/tmp/archie-pr-integration-target`, in coordinated
exclusive Cargo slots. No dependency/model downloads were needed.

- RED: `cargo test --locked -p archie-memory --test inference` initially stopped
  because the shared manifest/lock had changed. Retried `--offline`; it reached
  the expected absent `archie_memory::inference` API compilation failure.
- GREEN: `cargo test --locked -p archie-memory --test inference`: 5 passed.
- First real PostgreSQL inference run exposed an invalid colon in the test
  fixture's Vault suffix; corrected the fixture to a digest suffix.
- GREEN: initial `cargo test --locked -p archie-postgres --test inference -- --ignored
  --test-threads=1`: 7 passed on real PostgreSQL18.6 before cross-task recall API
  was added. Includes reopen, scope/session isolation, migration, stale writer,
  failure/cancellation and trigger-injected receipt/outbox rollback.
- GREEN: `cargo test --locked -p archie-postgres --test persistence -- --ignored
  --test-threads=1`: existing 8 real database regression tests passed.
- GREEN: `cargo test --locked -p archie-memory -p archie-postgres -p archie-state
  -- --test-threads=1`: 149 passed, 71 explicitly ignored database cases.
- `rustfmt` on owned Rust files and `git diff --check`: clean at that point.

Final targeted proof after integration API completion and scoped self-review:

- Behavioral RED: `cargo test --offline -p archie-postgres --test inference
  account_sessions_are_isolated_and_focus_domain_binding_is_immutable -- --ignored
  --exact --test-threads=1` failed because a known session ID could be copied into
  a new account binding. Removed account/runner from external session identity
  hashing and enforced the entire stored scope on reuse; the updated suite passed.
  The offline invocation refreshed dependency edges after shared manifest edits;
  subsequent checks use `--locked`.
- Behavioral RED: the rollback test with `limit=1` returned no result while the
  older root was still on the active lineage. Moved exact active checkpoint IDs
  into the SQL predicate before LIMIT for task/context/session reads.
- Final GREEN: `cargo test --locked -p archie-postgres --test inference -- --ignored
  --test-threads=1`: **8 passed**, including cross-task context recall, copied
  session rejection, active-root recall under limit1, and tampered scope/task index
  rejection. Scope is selected in SQL before payload decoding and checked again
  against the decoded binding and index/digest.
- GREEN: `cargo test --locked -p archie-state --test inference -- --ignored
  --test-threads=1`: **1 passed** on the real extension-equipped PostgreSQL18.6.
  Drops/reopens StateStore and verifies binding, receipt, checkpoint, outbox,
  across-task context and opaque-session recovery without cross-account access.
- Final GREEN: `cargo clippy --locked -p archie-memory -p archie-postgres
  -p archie-state --all-targets -- -D warnings`: exit0.
- Final owned `rustfmt --check` and `git diff --check`: clean. Original v1
  `archie-postgres/src/schema.sql` has no diff.

Database fixture is root-owned `/var/tmp/archie-router-pg-20261021` on port55439;
only newly reserved `archie_test_*` schemas are used/cleaned by new tests. Exact
TimescaleDB2.29.2 and vector0.8.6 were available. Vector was enabled by this agent;
root then restarted its own cluster with Timescale preload and enabled Timescale.
No production database or cluster lifecycle was touched by this agent.

Scoped self-review found and corrected the session-identity and pre-filter LIMIT
issues above. No unresolved storage-contract issue remains from that review.
Core still supplies the trusted account/domain identity and fresh policy/time
constraints; the store does not authenticate a caller or authorize a model/tool.

Root integration still owns the actual filesystem Vault/service roundtrip and
artifact existence/digest admission before commit. Provider authentication,
downstream cloud privacy enforcement, actual models and hardware remain outside
these store tests. No commits were made by the persistence agent; only its
explicit owned paths are staged, leaving all other shared staging intact.
