# Personal/work integration implementation plan

> Execution: inline with superpowers:executing-plans; no delegated agents.

**Goal:** Connect domain-scoped pillar context to the existing cognitive loop and
enterprise boundary without replacing authority or memory protocols.
**Architecture:** Vault owns domain validation; Cerebra carries the domain; a
bounded pillar context builds observations; Enterprise preserves work attribution.
**Tech stack:** Existing Rust workspace, serde, thiserror; no new runtime service.
**Spec:** ../specs/2026-10-11-personal-work-integration-design.md

## Constraints and review focus

Preserve legacy records and APIs where practical, never decrypt a mixed request,
never infer work scope from message text, never grant tool authority from context,
never report a stub transport as delivery. Domain identifiers are bounded opaque
segments. Reject namespace-prefix collisions and cross-project references.

## Tasks

- [x] Vault: add validated ContextDomain and domain-aware ContextRequest; preflight
  references before disclosure; carry domain into capsule receipts. Tests in
  archie-vault/tests/domains.rs must fail before implementation and pass afterward.
- [x] Cerebra: propagate optional domain in Observation, validate before utility
  selection, and add bounded PillarContext to build local-first observations.
  Test task/context mismatch, mixed projects and end-to-end scoped capsule.
- [x] Workstation: preserve Enterprise ingress quarantine while marking work
  context; reject personal-domain egress and reject unimplemented remote sync
  instead of claiming successful delivery. Test adversarial labels and legacy
  egress compatibility.
- [x] Verify existing Vault/Cerebra/Workstation/REM suites; review diff and record
  concrete remaining runtime wiring. No cloud sends, service deployment or branch merge.

## Execution ledger

Ruling: reuse the existing clean linked worktree feat/unify, which already includes
REM and application bridges; do not merge its whole history into the older main.
Ruling: user explicitly waived repeated implementation approvals; proceed inline.

Ruling: inference/provider changes appeared concurrently in this worktree. Preserve
them and do not attribute or revert them. One transient compilation failed before
their manifest dependencies were updated; validation was rerun on the updated tree.

Ruling: reserve personal/ and work/ for explicit scoped retrieval. Merely omitting
the domain cannot bypass the new gateway boundary; other historical namespaces
remain compatible. No existing stored records were renamed or migrated.

Review: found inconsistent quarantine parsing between cognitive preparation and
egress. A failing test demonstrated that TRUE released export; the shared rule
now accepts only explicit false or historical absence as not quarantined.

Validation: final combined run exited 0: Cerebra 52, Vault 31, Workstation 27,
REM 18, totaling 128 passed, 0 failed, 1 ignored (real packaged Linux FreeCAD).
Fifteen new tests exercise domains, pillar context and the enterprise boundary.
The actual HTTP board also refuses personal task moves and keeps protected
domain/quarantine/privacy labels despite adversarial ingress metadata.

Completion here is the scoped context/enterprise slice. The live producers,
autonomous runtime, REM recorder and external messaging remain the explicit
pending work in docs/PERSONAL-WORK-AGENT.md; do not report full agent integration.
