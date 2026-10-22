# Provider catalog and native transport slice

Base: `bca8473f7d628a43d937cfca99509c12caaf850f`, remote verified as
`PentaVM/ArchieOS-Nvidia`; implemented in the shared `codex/router-memory-harness`
checkout. Ownership is runtime sources/tests/manifest and the workstation provider
compatibility re-export. No excluded PR, Core, CLI, harness, GUI, WSL generation,
GPU, model download or training source was changed by this slice.

## Delivered interfaces

- `archie_model_runtime::provider` owns the existing `ProviderSpec`,
  `ProviderProtocol`, `ProviderConfigError`, `builtin_provider`, `load_providers`
  and `save_provider`. `archie_workstation::provider` re-exports this module.
- `ProviderSpec::endpoint(&self) -> Result<ProviderEndpoint, ProviderConfigError>`
  chooses native versus compatible wire format. Existing `openai_endpoint()`
  continues to refuse Anthropic.
- `ProviderSpec::discover(&self) -> Result<Vec<RemoteModel>, ProviderConfigError>`
  explicitly requests the provider catalog. This is async and does not assert
  entitlement. Configured names remain usable without a discovery request.
- `ProviderSpec::descriptors(&self, discovered: &[RemoteModel]) ->
  Result<Vec<ModelDescriptor>, ProviderConfigError>` merges validated names;
  path-safe IDs are SHA-256 identities of provider ID plus native model name.
- `provider::load_catalog(models: impl AsRef<Path>, providers: impl AsRef<Path>) ->
  Result<ModelRegistry, ProviderConfigError>` merges manifests and configured
  names, refusing duplicate provider or descriptor identities.
- `Routed::from_paths(models: impl AsRef<Path>, providers: impl AsRef<Path>) ->
  Result<Self, RoutedError>` makes that catalog callable by the existing router.
- `ProviderEndpoint::anthropic(base_url: &str, credential_env: Option<String>) ->
  Result<Self, RuntimeError>` always means Cloud and requires HTTPS plus a
  credential source. A test-only private endpoint literal supplies loopback
  fixtures without weakening the production constructor.

Generated registrations have basic text response capability only, quality 0,
unmeasured latency, unknown retention, and explicit labels for unverified
availability/context/capability assumptions. Context/output limits (4096/1024) are
budgets, not discovered capacity claims. Unknown cost uses a conservative signed
TOML-compatible integer sentinel with `cost-status=unknown`; it is not presented
as free or as a measured price. Default routing with no cost ceiling remains
eligible. No reasoning, vision, tool capability, retention verification or model
access is inferred from an ID.

The existing client now sends Anthropic Messages JSON, native system/tool blocks,
`x-api-key` and `anthropic-version`, and maps observed JSON usage/text back into
the common response. Native SSE assembles split tool inputs, withholds reasoning,
and requires a valid terminal lifecycle. Native and OpenAI-compatible transports
share incremental response limits and request budgets. OpenAI SSE now rejects
truncated responses and complete oversized lines. A local label requires a
loopback host even with HTTPS. Deadlines enclose the entire HTTP operation,
retry/backoff/body/stream, and the outer routed fallback loop; dropping a future
also drops its active request. Streams are never retried after output begins.

## Red / green evidence

All Cargo commands ran CPU-only in `ArchieOS-Operator-RTX3070-Test`, one Cargo
process at a time with the root coordinator's slot. Shared flags:

```text
CARGO_TARGET_DIR=/var/tmp/archie-pr-integration-target
CARGO_BUILD_JOBS=1
CARGO_PROFILE_DEV_DEBUG=0
CARGO_PROFILE_TEST_DEBUG=0
CUDA_VISIBLE_DEVICES=
```

1. `cargo test --offline --locked -p archie-model-runtime --test provider_boundaries`
   initially failed 2 of 3 actual assertions: arbitrary HTTPS host accepted as
   Local, and a 60ms operation with Retry-After consumed 3.004s. Unknown-length
   incremental response rejection already passed and was retained.
2. Catalog and native tests were written before their APIs. Separate RED
   compilations reported the expected absent catalog methods and native
   mapper/decoder/parser symbols (not behavioral test failures).
3. Full runtime run after implementing those APIs: native unit tests all passed;
   three new boundary assertions failed for truncated SSE, complete oversized SSE
   lines, and unbounded tool-chat input. These were then fixed.
4. Catalog conflict roundtrip exposed a real TOML serialization failure for the
   initial `u64::MAX` unknown-cost sentinel. It was replaced with `i64::MAX as u64`.
5. Final `cargo test --offline --locked -p archie-model-runtime -p archie-router`:
   **91 passed, 0 failed, 2 ignored** (runtime 53 passed/2 ignored; router 38 passed).
   Includes Messages JSON and SSE over real loopback TCP fixtures; split native
   tool inputs; provider registration/discovery identity/conflict tests; privacy;
   overall retry, stream and fallback deadlines; unknown-length output; and SSE
   and streamed-input bounds. The two ignored tests require real Nebius access.
6. `cargo clippy --offline --locked -p archie-model-runtime --all-targets -- -D warnings`:
   exit 0. Scoped rustfmt and `git diff --check` completed successfully.

The first RED build was interrupted and restarted with the exact debug=0 baseline
flags after the coordinator noted the difference. No other agent's Cargo process
was interrupted. Offline Cargo metadata added runtime sha2/toml/tempfile dependency
entries to `core/Cargo.lock`; the shared lock is left for root staging.

## Authentication and limits

Native Anthropic here means API authentication, not a claim of subscription
entitlement. No real provider request, OAuth/token-store read, live account login,
hardware, model quality, latency or deployment validation was performed. Fixture
timings test deadline enforcement only. Existing streamed-event API does not
report usage; JSON completions retain observed usage. Model catalog discovery is
explicit and does not silently add new endpoints or perform inference.

Owner-operated Codex uses its official managed account/session protocol; this
slice does not implement that process adapter. Third-party products must use the
provider-supported authentication route. Anthropic's SDK documentation requires
prior approval to offer claude.ai login/rate limits through a third-party product;
API authentication does not depend on that approval.

Primary references:

- https://platform.claude.com/docs/en/build-with-claude/streaming
- https://platform.claude.com/docs/en/api/overview
- https://code.claude.com/docs/en/agent-sdk/overview
- https://learn.chatgpt.com/docs/app-server
- https://developers.openai.com/siwc/token-sharing-open-source/codex-app-server

Root owns live service/Core/CLI wiring, actual authenticated inference proof,
session/domain persistence and aggregate workspace regression/review. No commit
was created by this implementer; shared index/lock integration belongs to root.

## Independent review fixes

Reviewed `_dev/verification/router-memory-harness/provider-review.md` against the
actual source at `a18bd22`. Both Important findings were reproduced and fixed;
the earlier 91-pass snapshot above remains historical evidence.

P1: reserved builtin cloud identities (`openai`, `nebius`, `cerebras`,
`openrouter`, `anthropic`, case-insensitive) reject Local/Fabric declarations,
including loopback proxies. Exact known cloud service hosts reject non-Cloud
locality even under a generic provider ID, with case/trailing-dot normalization.
Generated OpenAI/Nebius descriptors retain their fixed cloud provider kinds so
registry mutation cannot relabel them as Fabric. Compatible-wire clients accept
these fixed identities, and their descriptor-selected endpoints retain configured
credential sources. Native Anthropic already remains Cloud.

Generic operator-admitted loopback local models and HTTPS fabric deployments
remain supported. Their physical backend cannot be inferred from the URL and no
hardware attestation claim was added. A configured HTTPS loopback cloud proxy
retains Cloud privacy gates; HTTP cloud proxies remain unsupported.

P2: native `stop_reason=refusal` is recognized as an explicit terminal outcome in
JSON and split SSE. Supplied text/JSON usage are retained; an empty refusal is also
terminal. `Answered` adds public `finish_reason: Option<String>` so Routed callers
can classify a semantic refusal instead of losing its status. Root accepted this
additive field and owns the unified service receipt classification. A refusal is
not converted into a transport failure or an automatic fallback attempt.

A private routed runner seam reuses the same production fallback loop and answer
conversion in the deterministic native HTTP fixture. The fixture asserts refusal
content/status/completion usage and that the next provider's real loopback TCP
listener receives no connection. Production HTTPS requirements are not relaxed
for this test.

Fresh regression evidence with the same coordinated CPU-only flags:

- RED `cargo test --offline --locked -p archie-model-runtime --test provider_catalog`:
  5 passed / 3 failed, reproducing cloud service host marked Fabric, reserved cloud
  identity marked Fabric and loss of fixed OpenAI descriptor identity.
- RED `cargo test --offline --locked -p archie-model-runtime --lib refusal`:
  3 failed. JSON refusal returned NativeProtocol, split SSE refusal returned
  Malformed, and the native routed fixture contacted fallback and exhausted its
  deadline instead of preserving refusal.
- GREEN `cargo test --quiet --offline --locked -p archie-model-runtime -p archie-router`:
  **98 passed, 0 failed, 2 ignored** (runtime 60 / router 38). Includes all seven
  review regressions and positive generic local/fabric deployment coverage.
- GREEN `cargo clippy --offline --locked -p archie-model-runtime --all-targets -- -D warnings`:
  exit 0. Its first run flagged test-module placement; moving that module to the
  end of its source file resolved the warning, and tests plus Clippy were rerun.
- Scoped rustfmt and `git diff --check` completed successfully.

Official refusal semantics:
https://platform.claude.com/docs/en/build-with-claude/handling-stop-reasons#refusal

This review follow-up made no live provider request, credential inspection, model,
GPU, excluded PR or hardware/deployment claim, and created no commit.
