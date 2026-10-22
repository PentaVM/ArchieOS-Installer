# Google Harness² integration — 2026-10-21

Scope: `PentaVM/ArchieOS-Nvidia`, branch `codex/router-memory-harness`, baseline
`bca8473f7d628a43d937cfca99509c12caaf850f`. Origin was verified before writing.
No excluded PR, IRIS/Wanda/Sentry implementation, GUI or WSL generation was edited.
No commit or shared index operation was performed by this implementer.

## Actual integration

`tools/harness2/vendor` contains 17 untouched upstream files (247,692 bytes), pinned
to Google `google-research/harness2` commit
`1cfe254e8d25b15f37048c3730134686abe60335`. The Git tree blob IDs and source SHA-256
digests are in `tools/harness2/upstream-manifest.json`; the worker verifies source
digests before import. Independent verification also recomputed all 17 Git blob
SHA-1 identities from raw bytes and matched the pinned Git tree. Apache-2.0 license
and original source notices are retained.
No dataset, benchmark installation, GPU package or model weight was downloaded.

The Python worker invokes actual upstream `schedule.run_task`, sequential
Propose–Probe–Compose, trivial final selection, `reflect_adoption` and `Library`.
It implements custom Archie adapters, rather than reproducing the method in Rust.
Upstream's prompt packs only recognize Codex/OpenCode surface vocabularies; the
custom substrate uses the OpenCode-shaped four text files with an
`archie_mediated` channel. It does not import or invoke the upstream CLI executors.

The production slice is `k=1`, sequential: four solver runs and three edit stages,
then one reflection, eight mediated calls total. The actual upstream ledger shows
`R=4/P=3`. Upstream final selection excludes baseline and cannot abstain; a separate
parent promotion callback decides whether to admit the full tree and lesson or
retain baseline. Results explicitly say `improvement_verified:false`.

## Implemented boundary

- `tools/harness2/worker.py`: strict bounded UTF-8 JSONL, source pin verification,
  data-only proposer/substrate/BenchmarkAdapter, shared token/cost/call/deadline
  budget, cancellation and fresh parent domain/Policy confirmation every stage.
- `tools/harness2/protocol.json` and README: exact version-1 wire contract and
  operator/runtime requirements. No worker endpoint or credentials exist.
- `core/crates/archie-workstation/src/harness2.rs`: typed asynchronous process
  bridge, trusted handler callback, separate default-deny promotion, environment
  clearing, incremental bounded frame reads, prompt/digest/evidence/domain/Focus/
  objective/Policy/reservation/stage dependency checks, total monotonic deadline,
  owned-child kill on cancellation/drop and result/commit-path validation.
- `core/crates/archie-workstation/tests/harness2_bridge.rs`: CPU subprocess fixtures
  for admission/frame/deadline failures and explicitly opted-in real upstream test.

The parent supplies fresh admitted context and executes each stage through the
normal selected model runtime. A successful handler reply must mean fresh trusted
admission; this module does not grant authority itself. Generated scripts, plugins,
hooks, subagents, host shell commands, Policy changes and guardrail edits are
rejected. Model weights remain fixed.

Full task/stage contexts and trajectories are staged privately on checked Linux
`/dev/shm` tmpfs and deleted on normal exit. Durable state holds complete adopted
trees, attributed lessons, evidence refs/digests and safe receipts. Namespace
includes exact authorization domain, task family, model/config digest and upstream
commit. A later task may reuse a lesson only when all supporting refs AND digests
are admitted again. Completed exact identities resume; changed input or terminal
failed/interrupted runs do not silently restart. Symlink ancestors and path escapes
are refused.

A library publication is readable only when a completed receipt commits the exact
manifest hash, so interrupted/orphan publication cannot train future context.
Resume also verifies the adopted tree bytes. No rendered private model prompt or
credential/Veil map is written into a durable record.

Unknown usage remains null in observed totals and consumes its complete reserved
budget. Reports above reservation are recorded and terminate further work. Root
must derive conservative reserves from the selected runtime and enforce them
before spending. Global budgets across deliberately new run IDs remain Core's
responsibility.
Upstream's default USD fields are omitted from the exposed method ledger: they
would otherwise falsely appear to report zero cost for the custom mediated
adapter. R/P counts and stage labels remain actual upstream values; observed and
charged monetary totals come only from the parent-reported `usage` contract.

## Verification recorded

TDD: initial Python tests failed against the absent worker. Initial integration
exposed the upstream surface-name restriction; the adapter was corrected without
changing vendored source. Crash/integrity/usage/staging regressions were observed
failing before the corresponding hardening passed.

Supported CPU runtime: CPython 3.13.16 standalone in the existing test WSL distro,
download size 35,370,434 bytes, SHA-256
`f326b7a386bf22def82e55dc3b95d2bef8b7c938b4c98ec32dd65433e7720b50` verified before
extraction under `/tmp/archie-harness2-python`. The distro's default Python3.14
was not used for upstream execution. No WSL generation/configuration was changed.

Latest Python suite:

```text
/tmp/archie-harness2-python/python/bin/python3.13 -m unittest discover \
  -s .../tools/harness2/tests -v
Ran 15 tests in 10.167s — OK
```

Coverage: actual upstream PPC `R4/P3`; complete second-task library tree/evidence
reuse; domain isolation; exact resume/changed-input refusal; terminal failure;
path/script/Policy edits; unknown/reservation-exceeding usage; reflection budget;
cancel; admission mismatch; hung parent; duplicate JSON; baseline refusal;
orphan publication; persisted-tree integrity; symlink ancestor; memory-backed
private staging. The external inference parent is deterministic synthetic data.

Rust bridge files were rustfmt-formatted. After root granted the shared Cargo slot,
the ordinary fixture suite passed 4 tests (1 explicitly ignored real-runtime test).
The opted-in real upstream bridge test initially failed on an incorrect assumption
that upstream's display fingerprint was full SHA-256. Source inspection established
it is SHA-1 truncated to 16 hex characters; the bridge was corrected without
changing Archie's full SHA-256 identity/manifest/artifact hashes. The actual
upstream bridge test then passed with eight mediated calls and default-denied
promotion.

Final combined Rust command, with configured supported Python:

```text
cargo test --offline --locked -p archie-workstation --test harness2_bridge -- --include-ignored
5 passed; 0 failed; 0 ignored; finished in 1.81s
```

Strict `cargo clippy --offline -p archie-workstation --all-targets -- -D warnings`
passed in 10.03s. Two own lints (large result enum and needless question mark) were
fixed. Root authorized one offline lock refresh after concurrent shared manifest
changes; `core/Cargo.lock` was left unstaged for root. All builds reused
`/var/tmp/archie-pr-integration-target`, one job, debug info disabled, GPU visibility
empty. No duplicate target was created. Cargo slot was released to the queued
provider implementer immediately after these checks.

The real upstream Rust test remains explicitly `#[ignore]` by default until the
operator selects a supported interpreter; this session explicitly ran it. Root
owns the shared module export, unified service, persistence receipt and CLI wiring.

These results establish integration and contract behavior. They do not establish
provider authentication, model correctness, paper benchmark gains, hardware,
measured model latency or deployment readiness.
