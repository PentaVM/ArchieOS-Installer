# Router / memory / Google Harness² — continuation

Owner request:24 hours autonomous implementation of the internal local-AI router, supported OpenAI/Anthropic plan connections, persistent memory and Google Harness², unified and tested. Start2026-10-21T09:06:15Z; deadline2026-10-22T09:06:15Z. A quota interruption consumed09:10..12:45UTC; resumed explicitly by owner. Do not invent completed work or stretch the deadline silently.

Repository PentaVM/ArchieOS-Nvidia, origin verified. Isolated managed worktree `C:\Users\Penta\.codex\worktrees\router-memory-harness\ArchieOS`, branch `codex/router-memory-harness`, baseline `bca8473`. Existing open PRs are EXCLUDED:38 Wanda,46 Sentry,47 IRIS,48 speech-floor. Do not edit/integrate these, their branches or IRIS/Wanda/Sentry/Claude-Xavi GUI/WSL, training processes or CUDA contexts. No GPU work needed. Personal/work/project authorization remains exact and precedes ranking/decryption.

Automation `router-local-suscripciones-y-harness-24-horas` is ACTIVE on this thread hourly until deadline. It must be removed on completion/deadline. First attempted create failed (missing destination); corrected create with destination=thread succeeded. There is no old Wanda monitor in this chat.

## Audit state

- ProviderSpec in workstation/provider.rs persists state/providers.toml but only CLI consumes it. Runtime/Core load model manifests. Config/discovery is not routable yet.
- HarnessSpec supports register/login/version only. UhpClient has no production invocation; it wrongly treats loopback transport as local inference. Unknown-length responses are buffered before the2MiB check. Process probes have no timeout/output bound.
- Routed/inference HTTP retries can exceed the documented total deadline. Local-labelled arbitrary HTTPS can misrepresent actual inference locality.
- StateStore/PostgreSQL already persists graph/events/execution/checkpoints/outbox. resume_context has no runtime caller. Domain binding is absent from graph/focus/checkpoint contracts; Cortex project ranking is NOT authorization. Reuse Vault exact-domain receipts and sealed data, do not ingest excluded voice JSONL.
- Google Harness² is ABSENT. Existing HarnessRouter/UHP is a different project. Primary upstream pin `google-research/harness2@1cfe254e8d25b15f37048c3730134686abe60335` and paper2610.14224. Need actual upstream orchestration with safe custom proposer/substrate; stock upstream executors disable sandbox/approvals and may not run unconfined. Upstream final selection lacks a baseline/abstention gate, so its 'adoption' is not proof of improvement.
- OpenAI official personal app-server has managed account/login/start, account/read, thread/start/resume and turn lifecycle; credentials stay in official Codex. SIWC for third-party apps is a different app-owned OAuth registration. Never read/copy token files. Anthropic personal CLI login is distinct from approval to offer claude.ai subscription access inside a third-party product; official SDK docs require prior approval for that. Native API transport is a supported separate route. On Windows PATH codex.exe and hermes.exe exist, claude not found; auth not yet checked.

## Work in progress

Baseline CPU test is exec session99094: cargo test --locked -p archie-router -p archie-model-runtime -p archie-memory -p archie-workstation. Output: `C:\Users\Penta\Archie-RTX3070-Handoff\local-verification\router-baseline.log`. One cargo job, CUDA_VISIBLE_DEVICES empty, debug disabled, existing `/var/tmp/archie-pr-integration-target` reused. Do not create another huge target; host C has only~9GiB free. Do not clean another agent's build cache.

Agents audited without edits/builds/models/secrets: subscription_router_audit completed; memory_runtime_audit completed; harness2_audit finishing detailed upstream report. Before dispatch, list live agents and reuse them. Reports are in this chat; persist focused handoffs next.

No product code has been edited yet. Design/plan and isolated branch creation are authorized by the owner's autonomous brief; no repeated design approval is required. Still distinguish missing interactive account access from implementation work and never claim real authentication/inference based on a version probe or fixture.
