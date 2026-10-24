# Wanda Context Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: use superpowers:executing-plans for root integration, with focused parallel workers on non-overlapping C++, collectors and UI scopes. Steps use checkbox syntax.

**Goal:** Connect Wanda's existing person graph to scoped live context, local reasoning,
voice and a usable relationship view.

**Architecture:** Existing C++ GraphStore remains the person-graph authority. Rust owns
bounded Vault evidence, requests routed inference and exposes advisory DTOs. Existing
capture producers gain a separate content channel; Sentry metadata stays intact.

**Tech Stack:** C++23, Rust workspace 1.88 floor, Vault, existing Routed/Axum, Kotlin
NotificationCompat, browser Manifest V3 and dependency-free HTML/CSS/JS.

**Spec:** ../specs/2026-10-12-wanda-context-design.md

## Global Constraints

- Nvidia repository only; preserve Core/Cortex/IRIS/Orbe/Policy/leases/REM admission.
- Domain comes from configured authority, never untrusted message content.
- Request 256 KiB; batch 32; text 2000 chars; IDs 192 bytes; queue 256; context 64 refs.
- Restricted model context stays local under existing router/runtime policy.
- No external outbound messages, purchases, calendar mutations or model training.
- No changes to Kimi's active tree, network adapters, global GUI or WSL generation.

## Review Focus

- Concurrent writers and crash recovery must retain both updates and the same Vault key.
- Same contact/event identifier across work/personal cannot disclose or overwrite data.
- Notification repost/group title and edited DOM bubble cannot silently misattribute text.
- Uncited model output, expired promotion and ambiguous contact signals remain unverified.
- Private tool results cannot be returned to a cloud voice model as follow-up context.

## Task 1: Preserve existing Wanda imports and lock Vault persistence

**Files:** Wanda bridge/tests from f890b32; vault_io and watcher from 6a977c4;
`archie-cerebra/Cargo.toml`, persistence/domain tests.
**Interfaces:** `open_vault(path) -> VaultGuard` with Deref/DerefMut to Vault;
`persist_vault(path,&VaultGuard)`; watcher APIs additionally consume ContextDomain.
- [ ] Import the existing Wanda code, not new parallel parsing/category rules.
- [ ] Write/run failing concurrent-process key/write and cross-domain watcher tests.
- [ ] Lock the complete transaction, create key privately once, read with caps, persist
  atomically; retain context references in ingestion reports for the caller.
- [ ] Run existing/new Cerebra and Vault suites; commit.

## Task 2: C++ graph transaction and projection

**Files:** `sentry/{include,src}/store/graph_store.*`, new Wanda projection files,
`sentry/src/cli/main.cpp`, relationship files/tests/CMake as needed.
**Interfaces:** `archie-sentry export-json [--all]` returns the spec projection; existing
CLI graph mutations hold a transaction lock. Tags use Profile traits.
- [ ] Write/run failing projection escaping/redaction, hypothesis traversal and concurrent
  graph mutation tests.
- [ ] Implement projection via GraphStore load/RelationGraph queries; add explicit partner
  and manager semantics without remapping existing types; preserve old graph loads.
- [ ] Guard load-mutate-save against concurrent CLI writers and crash-created stale locks.
- [ ] Run C++ graph/profile/relationship/Wanda suites and real CLI export tests; commit.

## Task 3: Wanda runtime/API and recommendations

**Files:** new `core/crates/archie-wanda`, workspace manifests; Wanda schema/docs fixtures.
**Interfaces:** scoped Config maps domain→graph path and device token→domain/source;
`POST /v1/events`, `POST /v1/query`, `GET /v1/view`; `archie-wanda` serve/ingest/query/view.
- [ ] Write/run failing token/domain/replay/cap tests and real loopback API tests.
- [ ] Consume bounded C++ projection through configured absolute binary + argv, with no
  local shell. Store source evidence encrypted and reconstruct PillarContext per task.
- [ ] Produce gift and deadline advice from explicit evidence, then model interpretations
  through Routed with strict reference/standing/expiry validation.
- [ ] Add min quality support in existing Routed API without changing other callers.
- [ ] Verify no private cloud request, unknown citations, expired price or invented facts;
  expose unconfigured/missing-model states accurately; commit.

## Task 4: Live content collectors

**Files:** `capture-agents/android` additive Wanda files and notification hook;
`capture-agents/browser` additive WhatsApp script/transport/config; tests and schema.
**Interfaces:** emit spec v1 content envelope to separate configured Wanda ingest endpoint.
- [ ] Write/run failing DOM fixture tests for incoming/outgoing/group/repost/draft cases.
- [ ] Implement visible-chat MutationObserver, bounded queues and explicit coverage.
- [ ] Extract Android MessagingStyle text/timestamps/senders with fallback, actor gate and
  dedup; preserve metadata events and existing schema unchanged.
- [ ] Run collector tests/builds available locally; record device/live-session gaps; commit.

## Task 5: Voice and relationship view

**Files:** voice's existing Senses tool dispatch and privacy tests; standalone Wanda view
assets plus optional read-only shell-bridge source extension owned by this integration.
**Interfaces:** same Wanda DTO for voice/read-only UI; private response requires local
voice session, otherwise no details are fed to follow-up inference.
- [ ] Write/run failing voice privacy and bounded-answer tests.
- [ ] Add Wanda tool using existing floor/cognitive continuation, not a second voice engine.
- [ ] Build graph/list, search/tag/domain filters, profile/source/recommendation detail;
  use real API, clear empty/unsupported states and labelled demo fixtures.
- [ ] Verify UI with fixture API and screenshots, voice regression suite; commit.

## Task 6: Integration, review and handoff

- [ ] Run affected Rust/C++/collector suites and end-to-end fixture flow.
- [ ] Independent whole-branch review; correct material findings with regression proof.
- [ ] Record architectural changes, tests and real hardware/session limits in `_dev` and
  Wanda documentation; push a concise Draft PR and attach it to this chat.
- [ ] Preserve Kimi/Xavi work; do not fold Sentry branch commits or regenerate WSL here.
