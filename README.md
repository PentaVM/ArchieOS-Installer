# ArchieOS NVIDIA Edition

ArchieOS is a persistent AI runtime for building a JARVIS-like personal intelligence
across software, devices, and the physical world. Its identity, memory, policy, and
authority live in ArchieOS; language and vision models are replaceable cognitive
resources selected by the router.

This repository is the clean NVIDIA/Nebius competition edition. It contains the active
Rust control plane, native C++ analytical and visual subsystems, PostgreSQL/TimescaleDB
state, physical-world contracts, ArchISO packaging, and a provider-neutral inference
boundary. It does not contain the retired Python/PySide6 desktop application.

```text
Owner / environment
        |
        v
  Archie Core + Policy ---- Memory / PostgreSQL
        |       |
        |       +----------- Sentry / devices / observations
        v
  Capability Router
        |
        +-- local or fabric workers
        +-- NVIDIA NIM
        +-- Nebius Token Factory
        +-- other compatible providers
```

## Build and test

Rust 1.88 or newer is required by the locked dependency graph:

```bash
cd core
cargo fmt --all -- --check
cargo check --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

The native components use CMake and Ninja:

```bash
cmake -S cortex -B build/cortex -G Ninja -DARCHIE_CORTEX_BUILD_TESTS=ON
cmake --build build/cortex --parallel 2
ctest --test-dir build/cortex --output-on-failure

cmake -S sentry -B build/sentry -G Ninja -DSENTRY_BUILD_TESTS=ON
cmake --build build/sentry --parallel 2
ctest --test-dir build/sentry --output-on-failure
```

Run an isolated node after building `archied` and `archie-cli`:

```bash
./core/target/debug/archied --root /tmp/archie-node --text-logs
./core/target/debug/archie --root /tmp/archie-node status
```

NVIDIA NIM and Nebius setup is documented in
[`providers/nvidia/README.md`](providers/nvidia/README.md). Credentials are read from
environment variables at request time and never stored in repository configuration.

## ArchieOS Workstation for WSL2

The workstation alpha packages ArchieOS as an Arch Linux WSL2 distribution with
systemd, a non-root `archie` session, a sealed Vault boundary, local Nemotron ASR,
replaceable API/harness adapters and FreeCAD. Windows drives and process interop are
disabled inside the autonomous runtime.

Build on the supplied Arch builder or another Arch Linux environment:

```bash
sudo ./scripts/wsl/build-rootfs.sh ./dist
```

Install with WSL 2.4.4 or newer:

```powershell
wsl --install --from-file .\dist\ArchieOS.wsl
```

For older hosts run `dist\install-archie-wsl.ps1`, which imports the reproducible
rootfs fallback. Then:

```bash
archie setup
archie doctor
sudo /usr/lib/archie/install-nemotron-speech
archie provider add nebius   # or ollama/vLLM on loopback
archie tools test freecad
```

The router sends only gateway-issued Context Capsules. `Restricted` work stays local,
`Sensitive` stays within the controlled fabric, and `Secret` reaches no model. Model and
harness credentials remain in environment/upstream login stores and are never embedded
in the image. See [the WSL build guide](docs/WSL-BUILD.md),
[the security boundary](docs/SECURITY-BOUNDARY.md), and
[the first-run guide](docs/FIRST-RUN.md).

The FreeCAD smoke demo creates a parameterized motor bracket and exports FCStd, STEP and
STL inside `/archie/workspaces/freecad-smoke`. This is a build fixture, not an inferred
professional design. User tasks with missing critical dimensions go through Cerebra's
clarification gate before policy can issue a workspace lease.

## Current status

- **Working:** Rust authority, policy, scheduler, model router, memory and PostgreSQL
  state; Cortex; Sentry contracts; native observatory; OSM building ingestion; ArchISO
  build profile.
- **Working in this edition:** OpenAI-compatible NVIDIA NIM and Nebius inference
  transport with locality and privacy enforcement; reproducible Arch WSL packaging;
  deterministic clarification; provider/UHP harness adapters; workspace-scoped FreeCAD.
- **Experimental:** camera/model workers, detailed world streaming, and physical device
  adapters that require external hardware or datasets; local Nemotron live microphone
  integration on target hardware.
- **Planned:** privileged Windows host bridge and production deployment automation.

See [ARCHITECTURE.md](ARCHITECTURE.md), [HACKATHON.md](HACKATHON.md), and
[`docs/SECURITY.md`](docs/SECURITY.md).

## License

Apache License 2.0 — see [LICENSE](LICENSE) and [NOTICE](NOTICE).

The Apache licence rather than MIT for the patent grant: this repository contains
device control, physical actuation and inference-routing work, and an explicit patent
grant is worth more to a contributor here than the shorter text would be.

Upstream data providers keep their own terms. `providers/` records the attribution each
one requires, and the registry refuses to display records from a provider whose
attribution is missing — this licence does not waive those obligations.
