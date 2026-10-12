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
- [ ] Baseline affected Rust tests (running).
- [ ] Imported Wanda bridge/watchers and persistence concurrency.
- [ ] C++ person projection and graph transactions.
- [ ] Runtime, live collectors, recommendations, voice and view.
- [ ] Verification, independent review, documentation and Draft PR.

Ruling: use latest main and import only Wanda dependencies from Kimi — keeps his network
work out of this PR and avoids implementing against a branch missing latest voice/GUI.
Ruling: graph UI is a Wanda surface/DTO; global Quickshell layout remains Xavi's scope.
Ruling: Restricted context remains local under the existing runtime; larger model means
an eligible configured model with sufficient quality, not a guessed availability claim.
