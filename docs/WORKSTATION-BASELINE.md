# ArchieOS WSL workstation baseline

## Source state

- Initial synchronized HEAD: `9a2ab7ba0d530936a985e1f7fcf6612603bc0421`
- Branch: `main`, equal to `origin/main` after `git fetch --all --prune` and
  `git pull --ff-only`
- Sheriff/Santiago commit reviewed: `9a2ab7ba0d530936a985e1f7fcf6612603bc0421`
- Host: Windows `10.0.26200.9445`
- WSL: `2.7.13.0`, kernel `6.18.33.2-2`, WSLg `1.0.73.2`
- Existing builder distro: `ArchieOS-Builder`, WSL2

## Existing architecture map

| Concern | Existing implementation | Baseline state |
|---|---|---|
| Cerebra | `archie-cerebra` pass sequencer and advisory interpreter | Working |
| Memory | `archie-memory`, PostgreSQL/state adapters | Working |
| Context selection | Vault gateway plus admitted utility learner | Working |
| Advisory inference | `archie-model-runtime` composed by Cerebra | Working |
| Policy and leases | `archie-policy` | Working |
| Execution | `archie-execution`, `archie-executord`, task runner/Landlock | Working on Linux contracts |
| Vault | Authenticated sealed entries, opaque references, ephemeral capsules | Working library; no workstation secret-provider setup |
| Speech/floor | Speech contracts, deterministic floor, local learnable estimator | Working baseline; no real ASR process |
| Router | Capability/privacy/locality model router | Working for registered models; no provider config or process harness adapters |
| Observability | Content-free cognitive and authority traces | Working |
| CLI | Read-only node commands plus enrolment | Working; workstation commands absent |
| Distribution | ArchISO profile and native installer | Working source contract; WSL profile absent |
| UI | Native observatory and display daemons | Present; WSLg path unvalidated |
| FreeCAD | Engineering manifest only | Runtime/tool adapter absent |
| Clarification | Floor can defer speech; no explicit clarification state machine | Absent |
| Host bridge | Device/capture contracts exist | WSL host import/export contract absent |

## Baseline validation

The first combined Cargo invocation exhausted host memory while linking parallel test
binaries (`LNK1102` plus aborted `rustc` processes). The same code passed when rerun
crate by crate with `CARGO_INCREMENTAL=0`, `CARGO_BUILD_JOBS=1`, and one test thread.
This is an environment constraint, not a test regression.

| Crate | Tests passed |
|---|---:|
| `archie-cerebra` | 35 |
| `archie-memory` | 133 |
| `archie-observability` | 113 |
| `archie-vault` | 26 |
| `archie-speech` | 95 |
| `archie-router` | 33 |
| `archie-policy` | 32 |
| `archie-execution` | 64 |
| `archie-model-runtime` | 9 |
| `archie-cli` | 16 |
| `archie-installer` | 61 |
| `archied` | 64 |
| **Total** | **681** |

No assertion failed in the serial baseline.

## Reusable foundations

- Context capsules are task-bound, expiring, non-serializable, and refuse mixed
  disclosure when one reference is not permitted.
- Vault entries are encrypted and authenticated at rest; credentials can only be
  obtained through the broker.
- Cloud routing fails closed for restricted and secret data and for unverified
  retention.
- Capability leases are scoped, expiring, use-limited, revocable, and separately
  constrained for physical actions.
- Worker specifications cannot express shell commands and default to no network.
- Floor decisions are deterministic and already reject invalid estimates silently.
- ArchISO tests already enforce non-root daemons, explicit package allowlists, service
  hardening, and no development content in release images.

## Missing workstation components

- Reproducible Arch WSL rootfs and `.wsl` package.
- WSL-specific systemd units, directories, default user, installer, and doctor command.
- Local secret-provider provisioning for service credentials.
- Nemotron Speech process adapter with partial/final events and a real local smoke test.
- Clarification decision/state machine and resume contract.
- Provider configuration/discovery and cancellable harness process adapters.
- Capability-scoped FreeCAD tool runtime.
- Explicit Windows host bridge contract.
- End-to-end voice-to-CAD validation and final package artifact.

## Workstation blockers and constraints

- The host began with roughly 1 GiB free on both `C:` and `D:`. `cargo clean` reclaimed
  6.1 GiB from the repository target directory. Large builds must remain serial and
  incremental compilation disabled.
- No FreeCAD executable or Nemotron Speech runtime/model was detected on the Windows
  host at baseline time.
- Real microphone and GPU behavior must be tested inside WSL/WSLg after the rootfs is
  provisioned.
- Provider and subscription logins require user-owned credentials and upstream CLIs;
  neither belongs in the image.

