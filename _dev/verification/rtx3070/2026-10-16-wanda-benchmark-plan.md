# Wanda Blind Social Benchmark Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans task by task, inline here. Steps use checkbox syntax; no delegation selected.

**Goal:** Collect comparable responses from exact Wanda/base9B/Bonsai2-27B/Opus5.5/GPT6.1Sol arms and produce auditable blinded social-decision results without training leakage.

**Architecture:** A standalone evaluator validates disjoint manifests, performs bounded model calls, creates anonymized rating packs and computes family-level paired reports. It runs outside the trainer and never provides sealed prompts, answer keys or feedback for checkpoint selection. Provider credentials remain outside artifacts.

**Tech Stack:** Python3.11+ standard library for collection/validation/packing; NumPy for deterministic resampling/permutation statistics. Direct documented HTTP protocols; optional provider SDKs only if their pinned versions make a tested requirement simpler. No Codex/Claude agent CLI as an API-arm substitute.

**Spec:** [approved benchmark protocol](2026-10-16-wanda-social-benchmark-design.md). [Trainer plan](2026-10-16-wanda-trainer-plan.md) supplies a candidate receipt later, not a dependency for public pilot collection.

**Owner amendment:** [local AI Arena and progressive curriculum](2026-10-16-wanda-arena-curriculum-design.md). Register Codex, Claude Code, IRIS and other genuinely discovered assistants; preserve separate direct-model, agent and IRIS tracks. No Arena readiness dependency is added before trainer parity/backward/initial updates.

Status: plan for owner review; no runner, sealed case set or scored results exist.

## Global Constraints

- Scope is PentaVM/ArchieOS-Nvidia; do not alter Claude/Xavi GUI/WSL or Core policy. Wanda has no new moral reward/filter.
- Exact arms only: unchanged9B, candidate on the same base, hashed local27B, `claude-opus-5-5`, `gpt-6.1-sol`. Missing access/model/candidate is `not_run`, not a substituted result.
- Public pilot24families×4variants=96prompts, permanently excluded from sealed. Initial sealed design120families×4variants=480prompts per arm; final sample-size decision precedes sealing.
- Sealed/private source cases and identity keys live outside the trainer worktree and Git; model-visible prompts exclude answer keys. Candidate freezes before any sealed inference. Reuse after seeing scores makes a set exploratory.
- Identical admitted evidence/objective/instructions, no tools/retrieval/memory, input ceiling4096tokens with recorded provider counts, generation cap4096tokens, visible answer<=600words, fixed documented reasoning settings. Truncation is reported without quality retries.
- Pilot remote spend capUS$2, later full-run capUS$25; conservative reservation before every request, no automatic full run. Local models load sequentially under measured resource admission.
- Family-clustered95% interval,10000bootstrap draws, seed20261016; pairwise scores win1/tie0.5/loss0; three requested competitor tests corrected with Holm, one-sided familywisealpha0.05.
- Independent blinded human raters establish primary strategic preference; model judges are secondary only. Cannot call a developer-authored/self-rated pilot blind.

## Review Focus

1. Provider reasoning exhaustion can look like an empty successful answer; Task2 records stop reasons/usage and never replaces it with a better retry.
2. Aliases/missing versions and an unavailable arm cannot become a win; Tasks1–2 validate returned identity and retain `not_run`.
3. Sealed causal variants/translations cannot inflate sample size or feed optimization; Tasks1 and4 use ancestry/family units and manifest hashes.
4. Anonymous filenames can still leak model IDs/request headers or stylistic identity; Task3 strips metadata, logs randomization and collects identity guesses after rating.
5. Rater disagreement, failed requests or absent forecasts must remain visible; Task4 preserves initial ratings, missingness and denominators instead of favorable filtering.

## Files and shared contracts

Create independent `training/social_benchmark/` with pyproject.toml, README.md, `src/archie_social_benchmark/` and tests. Console script `archie-social-benchmark` maps cli:main. No imports of trainer internals; consumes serialized validated candidate receipts only. Use the approved JSON format/schema version1 with canonical content digests.

- `contracts.py`: `Case`, `Arm`, `RunConfig`, `Attempt`, `Response`, `Rating`, `Report`, `ManifestReceipt`, `PackReceipt`, `RatingSet`, `CostBound`, `Reservation`, `ReportReceipt`, typed validation/collection errors. Task1 introduces the shared dataclasses; later tasks implement their behavior. CostBound contains maximum billable input/output tokens and dated unit rates; Reservation contains request id/reserved amount/settlement status. ManifestReceipt contains phase/families/count/digests/readiness; PackReceipt contains anonymized pack digests and a private identity-map handle; RatingSet keeps original/adjudicated/missing records; ReportReceipt binds report/data/code/config digests.
- `manifests.py`: family/split/rights/overlap validation and sealed readiness receipt.
- `providers.py`: OpenAI/Anthropic protocol adapters with private credential resolution; `local.py`: pinned native inference collector.
- `arena.py`: capability/locality inventory and bounded autonomous queue; `cli_agents.py`: versioned Codex/Claude Code subprocess adapters; `iris.py`: replay/perception/action adapter reusing the existing Arena/IRIS contract. Tests: test_arena.py, test_cli_agents.py, test_iris.py. These agent/system arms do not substitute for direct-model comparison arms.
- `collect.py`, `cli.py`: immutable run/attempt receipts, cap reservation, transport retries, resumable collection.
- `blind.py`, `ratings.py`: rater packs, identity-key separation and independent rating import.
- `statistics.py`, `report.py`: clustered inference, calibration/robustness breakdown and readable export.
- `configs/public-pilot.toml`: public bounded settings; `cases/public-pilot.jsonl`: nonsealed owned examples. Actual sealed manifest remains external.

`Case` contains id, family/variant/stratum, domain, public messages/action options, ancestry and manifest digests; private keys/outcomes are in a separate evaluator-only record. `Arm` contains exact requested id, required returned id, base/adapter hashes where local, runtime/template and frozen settings. `Response` records visible answer, stop/truncation/failure state, request/response digests, returned id, actual settings, timing/usage/cost. Internal reasoning content is not needed in public artifacts; retain provider usage/status without exposing private reasoning. `Rating` records pack/item/rater ids, preference and six anchored dimensions, decisive facts, disagreement/adjudication and identity guess.

## Task1: Manifests, public pilot and isolation

**Files:** contracts.py, manifests.py, configs/public-pilot.toml, cases/public-pilot.jsonl, tests/test_manifests.py.

**Interfaces:** `validate_manifest(path: Path, phase: Literal['pilot','sealed'], forbidden_digests: set[str]) -> ManifestReceipt`; `validate_candidate(path: Path) -> Arm`. Build receipts include exact document/protocol/candidate/source digests. No trainer-facing sealed loader.

- [ ] Write failing tests for exact/near-duplicate translations, source-family crossing, missing variants, duplicate cases, changed protocol after sealing, model substitution and private answer keys in public messages.
- [ ] Implement canonical manifests and ancestry/normalized-text overlap checks; report suspicious near matches for independent review rather than claim automatic detection proves uncontaminated pretraining. Construct24owned public families across the five strata, clearly labelling all as development-only. Use spec worked examples only here.
- [ ] Run `PYTHONPATH=src python -m unittest tests.test_manifests -v` from training/social_benchmark. Seal mode requires independently reviewed cases/keys, candidate-freeze digest, rater availability and sample-size decision; no fake readiness from empty folders. Commit only public cases/config/tests.

## Task2: Exact model collectors and bounded calls

**Files:** providers.py, local.py, collect.py, cli.py, tests/test_collectors.py.

**Interfaces:** `collect(case: Case, arm: Arm, config: RunConfig) -> Response`; `reserve_cost(arm: Arm, upper_bound: CostBound, ledger: Path) -> Reservation`; CLI `check-access`, `collect --phase pilot|sealed`, `status`. Credentials are resolved from established environment/secret handles without logging values; absent credentials produce an explicit access dependency.

- [ ] Write failing transcript tests for wrong returned model,401/404access error,429/5xx retries (max2), blank answer with reasoning-cap stop, malformed usage, cost cap, crash after reservation, content failure without retries and resumed run accidentally repeating a charged request. These tests validate transport behavior, not model competence.
- [ ] Implement documented OpenAI Responses and Anthropic Messages adapters after reading current official reference docs. Respect Opus5.5 always-on adaptive thinking/medium effort and Sol medium reasoning without unsupported temperature/tool settings. Validate IDs via provider model metadata and one bounded public response; no aliases silently changing arms. Reserve worst-case budget with dated rates and token upper bounds before dispatch; unknown billing/token bounds reject paid dispatch. Reconcile ambiguous timeout spend conservatively rather than freeing its entire reservation.
- [ ] Implement local HTTP collector that verifies controlled native process/model/template receipt and loads one model at a time; do not touch normal8081/current.gguf. Supply same inputs and reasoning conditions as declared. CLI does not execute advice or model tool proposals. Preserve all attempts, content failures and cap exhaustion.
- [ ] Run `PYTHONPATH=src python -m unittest tests.test_collectors -v`. When credentials/access are actually available, perform at most2public prompts per remote provider underUS$2 initial cap and save real receipts. On this PC verify9B/27B sequential loading under actual budgets; record CPU offload as a distinct latency condition. Unavailable arms remain not_run. Commit code/tests/public receipts without credentials.

## Task2A: Local-use AI Arena adapters and autonomous queue

**Files:** arena.py, cli_agents.py, iris.py, configs/local-arena.toml; tests/test_arena.py, tests/test_cli_agents.py, tests/test_iris.py. Extend contracts.py with CapabilityReceipt, AgentInvocation, QueueReceipt and IrisCase. These define adapter/version, execution host/inference locality, supported track, actual model/weights identity, tool/context budget, executable/endpoint evidence, availability/reason and run/case receipts. No credentials in those records.

**Interfaces:** `discover_assistants(config: Path) -> list[CapabilityReceipt]`; `run_cli_case(case: Case, arm: Arm, config: RunConfig) -> Response`; `run_iris_case(case: IrisCase, arm: Arm, config: RunConfig) -> Response`; `run_arena_queue(cases: Sequence[Case | IrisCase], capabilities: Sequence[CapabilityReceipt], config: RunConfig) -> QueueReceipt`. CLI commands `arena inventory`, `arena run`, `arena status`, `arena cancel`, `arena resume`. Task2A is independent of the trainer's initial numerical proof.

- [ ] Write failing tests for missing Claude Code, Codex executable with unknown/auth-required model, an unexpected cloud inference locality, a detected model silently falling back, wrong IRIS weights, IRIS action output mis-scored as social prose, and tools used under a claimed no-tools condition. Inventory must retain every discovered assistant and show unavailable/not_applicable explicitly. No simulated output counts as an inference receipt.
- [ ] Implement registry adapters for native Bonsai, Codex CLI, Claude Code CLI and IRIS HTTP/installed Arena. Discover from declared executable lookup and existing registered local providers, without crawling private histories/auth files. Read installed `--help`/`--version` before constructing argv. Codex supports noninteractive `exec`/JSON events; Claude uses documented print/output modes when actually installed. Pin actual version/model, agent scaffolding/tool traces and locality; never enable bypass flags or use shell-built argv. Enforce ephemeral isolated case workspaces, bounded stdin/stdout/stderr/deadlines and no access to source repo, user's home or sealed keys. If CLI controls/environment cannot enforce a condition, mark that condition incompatible rather than equate it with the direct-model track.
- [ ] Reuse existing IRIS observation/action endpoints and `archie-arena-live` Stage gate for appropriate admitted replay/action tasks. Do not invent a text-chat endpoint or compare a control head's output with social reasoning. An optional IRIS+planner arm names both components and their complete resource/cost budgets. Preserve source/domain admission and existing execution leases. Separate perception/action/recovery metrics from social strategic metrics.
- [ ] Implement a bounded resumable queue with per-arm/case manifests, one GPU owner, owned-process cancellation, immutable outcomes and time/token/tool/cost ceilings. It may automatically advance through ready cases; it cannot train/promote models, retry bad answers, grade itself or start an unbounded run. Missing credentials/tool capability pauses only affected arms, not trainer parity/backward or other ready tracks.
- [ ] Run `PYTHONPATH=src python -m unittest tests.test_arena tests.test_cli_agents tests.test_iris -v`; then one public nonsealed case for each actually ready adapter. Record real requests/identities/failures, not availability alone. Existing Windows Codex discovery is not authentication proof; currently absent Claude remains unavailable until installed/configured. Commit adapters/tests/receipts separately and preserve old Arena task/action behavior.

## Task3: Independent blinding and ratings

**Files:** blind.py, ratings.py, tests/test_blinding.py; README.md rating instructions.

**Interfaces:** `make_rating_pack(responses: Sequence[Response], cases: Sequence[Case], evaluator_root: Path, seed: int) -> PackReceipt`; `import_ratings(files: Sequence[Path], pack: PackReceipt) -> RatingSet`.

- [ ] Write failing tests for provider metadata leakage, identity key accidentally included, unbalanced left/right ordering, identity-aware raters, self-rating, changed answer after pack freeze, missing second rater, duplicate rater and loss of initial disagreement.
- [ ] Implement anonymous items with immutable answer content and separate private identity keys. Randomize/balance pair order; do not rewrite substantive text to hide a poor answer. Provide six0–4 anchored dimensions from the spec and paired preference with decisive-fact notes. Ask identity guesses only after judgment. Require two independent raters plus recorded adjudication for disagreements, preserving original judgments.
- [ ] Run `PYTHONPATH=src python -m unittest tests.test_blinding -v`; manually inspect one public pack and its identity-key directory boundary. Log unresolved independence/anonymity limitations rather than label incomplete ratings blind. Commit code and public instructions only, never sealed keys.

## Task4: Clustered reporting and calibration

**Files:** statistics.py, report.py, tests/test_statistics.py, tests/test_report.py.

**Interfaces:** `paired_results(ratings: RatingSet, responses: Sequence[Response], seed: int = 20261016) -> Report`; `write_report(report: Report, directory: Path) -> ReportReceipt`.

- [ ] Write failing tests on independently hand-calculated all-tie/all-loss/all-win and mixed families, correlated variants, Holm adjustments, undefined calibration, missing arms and network failures. Repeating a variant must not increase the number of independent families. No inference of superiority from positive training loss change, schema rate or mean without adjusted tests.
- [ ] Implement family-cluster bootstrap10000draws and a paired family-level sign-permutation test with exact enumeration when feasible, otherwise100000MonteCarlo draws/fixed seed. Report wins/ties/losses, paired estimates/95% intervals, adjusted tests and stratum effects. Overall claim requires all three requested comparisons to pass; unchanged9B control is reported separately. Report pilot-based sample-size choice before sealing, without optional stopping.
- [ ] Add Brier/reliability only for defined resolved forecast events; mark unresolved/subjective intent labels not applicable. Report decision metrics, consequences, relational costs, noise/cosmetic invariance, causal responsiveness, novel families, citation inventions, format failures, resource/cost and missingness separately. Preserve full attempt/rating denominators and disagreement evidence; no opaque composite.
- [ ] Run `PYTHONPATH=src python -m unittest tests.test_statistics tests.test_report -v`. Check report labels distinguish synthetic simulation, public development pilot, blind held-out and real prospective observations. Commit code/tests and a prominently synthetic report fixture only; it is not a model result.

## Task5: Real pilot, sealed readiness and evidence handoff

**Files:** README.md and configuration/receipts only; no new algorithm here.

- [ ] Execute the public pilot twice with frozen feasible settings when actual arms/access/budget permit. Record per-arm failure/variability, cost, ceiling exhaustion and local resource measurements. A missing candidate may collect baselines; it cannot stand in for Wanda.
- [ ] Select final sample size/minimum relevant effect from pilot variability before sealing. Obtain independently authored/reviewed sealed case families and independent raters, freeze protocol/source/candidate digests, and verify they are outside trainer access. Neither the coding agent nor an evaluated model secretly authors the sealed answers and declares them unbiased.
- [ ] Run sealed collection once only when readiness is genuine. Import blinded ratings and report all outcomes, including inconclusive results. Do not use sealed feedback for LoRA expansion or checkpoint choice. Any later optimization requires a new untouched set.
- [ ] Commit only publishable anonymized receipts with proper rights; retain sealed prompts/keys privately. Deliver exact model/version/config/data hashes and reproducible analysis instructions. Do not claim beating any competitor before that competitor actually ran and the declared tests passed.

## Self-review and handoff

Spec coverage: identity/isolation Task1; reasoning/paid caps/transport/local conditions Task2; local CLI agents and IRIS/system tracks Task2A; independent blind ratings Task3; all declared quality/statistical dimensions Task4; real feasibility/sample-size/sealed claim gates Task5. Missing provider access, candidate parity and independent raters are genuine external prerequisites; they do not prevent implementing or testing the public collection pipeline. Never combine agent and direct-model results to support the original three-model superiority claim.

Execution remains inline here. Owner review of this plan and the trainer plan is the final workflow prerequisite before product implementation. Existing Core/IRIS/GUI/WSL processes are outside this evaluator's authority.
