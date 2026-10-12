# Wanda implementation ledger

Base: main ac9c1d2. Branch: codex/wanda-context. Owner: Codex.

User scope: complete Wanda before Sentry; Kimi owns Sentry. Reuse existing person graph,
context domains, Vault, Cortex/router and voice. Full-chat authorization already supplied;
design/plan checkpoints are recorded here without redundant approval pauses.

Audited: C++ PersonGraph/GraphStore/Profile/RelationGraph/Wanda exist and must stay the
authority. Kimi's new Wanda bridge/watchers are branch material. Stage is not a full-host
capture implementation. Live metadata agents drop bodies. Voice currently forwards plain
tool results to its current model; private Wanda results need an explicit boundary.

Progress:
- [x] Existing architecture and ownership audit.
- [x] Written design and plan, inspected for authority/domain/source consistency.
- [x] Affected Cerebra suite: 78 passed; ignored worker is invoked by its process test.
- [x] Imported Wanda bridge/watchers; Vault OS transaction, scoped watcher references.
- [x] C++ person projection and graph transactions: fresh CTest 18/18.
- [ ] Runtime, live collectors, recommendations, voice and view.
- [ ] Verification, independent review, documentation and Draft PR.

Ruling: use latest main and import only Wanda dependencies from Kimi — keeps his network
work out of this PR and avoids implementing against a branch missing latest voice/GUI.
Ruling: graph UI is a Wanda surface/DTO; global Quickshell layout remains Xavi's scope.
Ruling: Restricted context remains local under the existing runtime; larger model means
an eligible configured model with sufficient quality, not a guessed availability claim.

2026-10-12 integration checkpoint:
- UI model suite 11/11 and browser collector fixture suite 4/4 freshly passed.
- Wanda store tests observed RED on unimplemented config/store, then 8/8 passed
  (one child worker ignored by ordinary discovery and invoked by the process test).
  The suite includes six concurrent processes, encrypted bodies, retention, collision
  detection, source binding and trusted work/personal routing.
- Recommendation validator observed RED, then 2/2 passed: unknown citations, asserted
  standing, unknown people, expired proposals and missing alternatives are refused.
- C++ subprocess integration test observed RED; implementation added, GREEN pending.
- Runtime/API are still in progress. Voice compilation was interrupted to prioritize
  Wanda's own integration tests; no speech or microphone verification is claimed.
- Earlier parallel workers reached their account usage limit. Root continues inline.
- Build cache exhausted C:; accepted `cargo clean` reclaimed 9.9 GiB. Subsequent builds
  disable dev/test debug symbols and incremental output through per-command environment.
- No new Wanda changes have been pushed, merged or included in Sheriff's WSL image.

Later checkpoint:
- Wanda runtime/API suites GREEN: 17 passed plus child worker invoked by its parent.
  Actual C++ subprocess, authenticated HTTP and Wanda voice client exercised. The
  inference server is synthetic; no accuracy/latency/GPU claims follow from this.
- Voice `--features wanda --lib`: 55/55 passed. Browser against actual Wanda executable
  and actual C++ CLI: passed pairing, profile/filter, work/personal switch and 390 px
  viewport; screenshots inspected under the system temporary QA directory.
- Proactive refresh observed RED when a cached proposal delayed a new message; fixed
  with per-domain generation coalescing and tested GREEN on two distinct live batches.
- Local proxy regression observed RED: proxy received a local request. Local clients
  now bypass environment proxies. Runtime 19 unit tests, 5 SSE tests, quality routing
  and parent/child proxy proof passed. A pre-existing rate-limit test advertised the
  wrong Content-Length; corrected its fixture to compute the body's byte length.
- Expired query/graph snapshots initially remained at rest. Scoped retention now
  removes Wanda's own events/query/graph ciphertext, preserving other pillars.
- Main advanced to 8e0cf22 through Sentry PR #36 while this work was isolated. Claude
  Draft #35 covers Rust IRIS and memory tests, with CUDA/text tower limitations. These
  PRs are not proof of release rebuild or hardware validation. Integration with latest
  main follows this checkpoint; independent review and hardware proof remain pending.

Ruling: typed references and inferred standing are necessary validation, but do not
prove the truth of generated prose. Real-model accuracy and subtle social interpretations
require evals and user review; current DOM tests establish fixture behavior only.
