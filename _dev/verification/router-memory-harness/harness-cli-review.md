# Harness runtime / CLI / Core review — 2026-10-21

Repository verified: `https://github.com/PentaVM/ArchieOS-Nvidia.git`, branch
`codex/router-memory-harness`, HEAD `a18bd226a15fdcc65f499af5a193acfba9c5ac53`.
Reviewed working-tree/untracked implementation relative to `bca8473`.
Scope: `harness_runtime.rs`, CLI `ai.rs`/`main.rs`, Core `ai_control.rs`,
`control.rs`, `operator_session.rs`, and supporting bridge/worker contracts.
This initial review used source inspection only: no Cargo, model, or network run.
Concurrent fixes change line numbers; anchors below name the exact expressions.

## Findings

1. **Important — leaf-only worker checks permit an untrusted import tree.**
   `core/crates/archie-workstation/src/harness_runtime.rs:68`, `:77`, `:78`
   call `trusted_file` only for configuration, interpreter, and `worker.py`.
   `trusted_file` at `:43–58` does not examine ancestor directories or the
   adjacent manifest/import tree. `tools/harness2/worker.py:162–169` trusts
   `upstream-manifest.json` beside the worker, verifies hashes supplied by that
   same file, and explicitly adds adjacent `vendor` to `sys.path`; imports occur
   at `:391–393`. A trusted worker/interpreter beside another user's writable
   manifest/vendor passes admission but can import that user's code under the
   daemon's OS identity. Writable path ancestors also allow replacement after
   the leaf check. `-I` and cleared environment do not protect an explicitly
   imported adjacent tree. Require safe ownership/modes and non-symlink ancestry
   for the complete installed tree, reject unlisted executable/import artifacts,
   and preferably anchor the manifest digest in trusted code. Root accepted and
   owns this correction; no exploit was executed.

2. **Important — numeric monetary reserves do not bound spending.**
   `harness_runtime.rs:315–320` creates `max_cost_micros: 12_000` and
   `per_call_cost_micros: 1000` for every selected model. The handler returns
   `cost_micros: None` (original `:190–191`, `StageUsage` construction), so the
   bridge retains those arbitrary reserves. Provider-created descriptors mark
   costs unknown and use conservative sentinel prices in
   `core/crates/archie-model-runtime/src/provider.rs:149`, `:182–185`; the fixed
   reserve ignores them. An expensive API request is therefore admitted under a
   fictitious 1,000-micro-unit reserve. Unknown actual cost is correctly null,
   but the charged monetary ceiling has no defensible relation to spend.
   Represent unsupported monetary budgets as null for unknown-price APIs and
   managed byte/time plans. Only derive a numeric reserve from trusted known
   prices and a conservative full rendered input / enforced output ceiling.
   Root accepted; follow-up implementation delegated to this reviewer.

3. **Important — global expiry previously erased the active attempt's terminal
   attribution.** Initial `harness_runtime.rs:239–256` selected the worker result
   and external cancellation only; only the latter sent `stop.send(true)`.
   `harness2.rs:837–839` times out by dropping the entire callback future. If
   Harness duration is shorter than the profile's per-call timeout, expiry drops
   `execute_for_account` before the terminal receipt commit, so the paid/started
   stage has no failed/cancelled receipt. Initial admission/account discovery also
   preceded creation of the run deadline. Root's concurrent change now starts a
   monotonic deadline at entry, bounds admission, signals cancellation at expiry,
   and drains the worker before dropping it (`harness_runtime.rs:270–285`,
   `:338–354`). Static inspection confirms the intended correction; a real
   PG/provider deadline regression remains required and is owned by another agent.

4. **Important — default API Harness output limit previously excluded every
   provider-created model.** Initial `harness_runtime.rs:214` chose
   `profile.max_output_tokens.min(4096)`, normally 2048
   (`ai_policy.rs:40`). Catalog-created models allow 1024
   (`provider.rs:173`); the stage copied 2048 into `InferenceRequest`, and the
   router rejects it before HTTP (`archie-router/src/lib.rs:295–298`). CLI has
   no Harness output-token override, so a newly configured normal provider could
   not run even stage one. Root now derives the minimum profile/selected-descriptor
   limit (`harness_runtime.rs:286–300`). Static inspection confirms the correction;
   run the actual Core/PG/worker fixture with default catalog limits, including
   later proposer prompts, to verify full rendered context compatibility.

## Other reviewed boundaries

The generic Core dispatcher denies AI; authenticated Linux peer credentials are
required on the operator path. Domain and allowed-runner checks precede dispatch,
with profile-owner/scope revalidation before disclosure. Wire requests cannot
supply process paths. The CLI never falls back after a connection has submitted a
request or a response is lost (`ai.rs:101–112`). Harness API model selection is
pinned; stage dispatch pins the account; managed limits are explicitly byte/time
bounds. The production handler does not override default-deny promotion.
No improvement, hardware, latency, deployment, or real-provider claim is made.

One residual identity limitation: the model/config hash includes upstream commit,
model descriptor/account (or the official-client label), but not the installed
Archie worker/adapter bytes. A subsequent trusted adapter update with the same
upstream commit may reuse a prior run identity. Bind a versioned adapter digest
when the worker installation is hardened, or explicitly invalidate the old cache
on adapter changes.
