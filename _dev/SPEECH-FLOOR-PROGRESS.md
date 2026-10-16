# Execution ledger — docs/superpowers/plans/2026-10-11-speech-floor-completion.md

Base: `0bd6405`. Worktree: `archie-nvidia-main-integration`, existing isolated main.
Owner explicitly authorized merge/push and asked not to repeat permission gates.

Pre-flight: ASR adapter supplies text/final to conversation; conversation uses the
existing SpeechSession/FloorPolicy. Bilingual CLI must preserve current RVC wrapper.

- Task 1 complete: fast-forward `feat/sentry-complete` to `aaf2071`; all other local
  branch heads are ancestors except `c9b3603`, which `git cherry` identifies as
  patch-equivalent. Current main already has the later persistent-memory integration.
- Task 2 implemented: pending bilingual tracked diff applied except CLI, which is
  adapted against current RVC code. Four source files copied explicitly. Source
  worktree untouched. No staged WSL or uncommitted Orbe render files imported.
- Task 3 implemented: additive ASR trait wraps native SDK; conversation builds without
  SDK to exercise the actual loop. No decoding or models replaced.
- Task 4: first six actual-loop replays, two receipt regressions and full speech/voice
  suite passed in `archie-speech-final.log`. RED logs: archie-floor-red.log,
  archie-floor-overlap-red.log, archie-speech-gate-red.log, archie-cancellation-red.log,
  archie-receipt-red.log, archie-playback-red.log, archie-tts-error-red.log,
  archie-shutdown-red.log. Initial baseline raced a manifest integration, so it is not
  a reliable baseline compile result; subsequent full speech/voice run succeeded.
- Final read-only reviewer found three Important edge cases: repeated acknowledgements,
  short overlap recognized after voicing stops, and worker consuming queue after TTS
  error. Accepted and fixed after reproducing all three. RED logs:
  `archie-review-regressions-red.log` (worker: 47 pass / 1 fail),
  `archie-review-overlap-red.log` (actual loop: 6 pass / 2 fail).
  Native Piper launcher Python dependency was also corrected.
- Final speech/voice proof: `archie-speech-review-final.log`, exit 0, 155 passed,
  0 failed, 0 ignored. Includes eight actual-loop replays, three receipt/worker
  regressions, language selection and existing ASR-independent tests. Code committed
  as `e5cf04a`. Bash syntax (three scripts), Python AST and diff check passed.
- Affected-module compilation was interrupted because disk space became low, not
  because tests failed. Recursive cleanup was rejected by automatic approval review;
  `cargo clean --target-dir` succeeded and removed only 2.5 GiB of build artifacts.
  Subsequent checks use `CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0`.
- Ruling: use portable deterministic replay for the actual loop; SDK/model/hardware
  validation requires its installed runtime. Cost: replay cannot establish real ASR
  accuracy, synthesis latency or acoustic echo cancellation.
- Ruling: preserve Claude's WSL/GUI and existing render modifications in source
  worktrees. This completion integrates voice and committed branches only.
- Later branch delivery: Claude published WSL Operator `b22625e` and GUI audit
  `586637d` while validation ran. User explicitly assigned branch merges to Codex,
  so completed artifacts were merged (`633f7ac`, `30978a0`); no GUI authored or WSL
  generated/installed. Proposal ADRs remain unaccepted. Operator omits voice and
  Observatory; its automount differs from isolated runtime and is not validated here.
- Affected Core/CLI/Sentry/LAN/State/daemon/Workstation proof:
  `archie-nvidia-affected-final.log`, exit 0, 318 passed, 0 failed, 84 ignored.
  Omitted integration cases require the PostgreSQL/fixture environment stated by
  the tests; passing portable suites is not proof of their real runtime behavior.
- Bridges stale generated-index regression reproduced, then regenerated (one line).
  `archie-bridges-final.log`: 55 passed, 0 failed, 0 ignored, exit 0. All executed
  suites total 528 passed / 0 failed / 84 ignored. PostgreSQL 18 exists but the
  harness-required TimescaleDB/pgvector extensions are absent in its installation.
- Static checks also pass for four imported WSL Bash scripts and installer PowerShell
  AST. No WSL setup/build/import or IRIS operation was executed.
- Same review seat supplemented only newly delivered WSL/GUI artifacts. One Important
  finding accepted: root builder deleted the host pacman cache. A static cleanup-target
  audit reproduced a target outside owned image/workspace variables; removing that
  single host cleanup made all four top-level cleanup commands pass the audit. Bash
  syntax and diff checks pass. Existing rootfs cleanup remains; no image was built.
- Supplement rulings: deployment, GPU/IRIS, package resolution, ABI, import, provenance
  and real Windows-drive access stay deferred/unverified. Operator's automount,
  shared-seat/emergency-stop and benchmark assumptions are distinct from production
  OS validation. GUI proposals remain unaccepted; no new authority service integrated.
- Last fetch delivered GUI prototype `4b282cd` after audit merge. It is unfinished
  Phase B work assigned to Claude/Xavi, so it stays on its branch; importing the
  completed audit did not authorize treating that prototype as a finished integration.
- Task 5 portable verification/integration/publication complete: normal push to
  origin/main and `git ls-remote` matched `b0e2e77eac8afefc1606917be55a8add11cb724a`.
  Working tree clean. This follow-up records that observed publication; its own
  publication is checked again before the final response. Native/hardware follow-up
  stays explicitly pending and no other worktree is removed.

## Reviewer declined-to-judge rulings

- Native feature compilation / SDK ABI: defer to real installed SDK in Claude's WSL;
  accepted limitation, no fake prefix or claim of native verification.
- Real ASR, bilingual quality, RVC, AEC and model latency: defer to reproducible hardware
  benchmarks in current voice handoff; synthetic boundaries cannot establish them.
- Browser playback: nominal pacing only; client playback ACK remains future work.
- Pulse drain interruption: blocking sink operations remain unverified; measure
  cancellation latency and server stalls on target environment.
- Piper package API / metadata: pinned PyPI releases and publishing source API read;
  both remote voice configs and model cards matched installer MD5, declared 22,050 Hz,
  and stated LibriVox public-domain / Unlicense recording terms. Weights, installation,
  voice quality and any additional downstream license obligations remain unverified.
- Sentry/systemd/ISO/database/devices: imported baseline and its audit remain explicit
  pending work; affected portable tests cannot certify deployment or paired hardware.
- Remote/branch/worktree evidence: executor verifies directly before publication.
- GUI and WSL generation: excluded by user, remain with Claude/Xavi.
- Adjacent preexisting model-failure handling and blocking tool callbacks: deferred
  existing behavior; no replacement authority introduced by this patch.
