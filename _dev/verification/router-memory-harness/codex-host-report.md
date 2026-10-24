# Managed Codex host diagnosis

Observed 2026-10-21 22:50–22:55 UTC. The remote was verified as
`https://github.com/PentaVM/ArchieOS-Nvidia.git` before writing this report.
No product source was edited and no model inference was run for this diagnosis.

## Finding

The Linux-to-Windows executable boundary in
`ArchieOS-Operator-RTX3070-Test` fails before the official Codex app-server emits
its first protocol frame. This reproduces with Windows `cmd.exe /d /c exit 0`,
so account authentication and Codex's JSON protocol are not the cause of this
observed failure.

The fixed stderr signature is `UtilAcceptVsock:250: accept4 failed 110`.
Windows executable interop returns exit 1 after approximately 10 seconds.
`WSL_INTEROP` is absent in the diagnostic invocation, and no control socket for
its current session is available. This narrows the observed boundary to WSL's
interop route/fallback. It does not prove the internal reason that route failed.

## Actual evidence

| Host and child | Configuration | Observed result |
|---|---|---|
| Native Windows Python → official Windows `codex.exe` | Exact Rust client per-child configuration | `initialize` and `account/read` succeed; ChatGPT auth and stable account identity are available |
| WSL Linux Python → same Windows `codex.exe` | Exact Rust configuration | EOF before initialization; stdout 0 bytes; stderr 59 bytes; exit 1; fixed WSL vsock-timeout signature |
| WSL Linux Python → same Windows `codex.exe` | Original Python-probe configuration | Same EOF, stdout 0 bytes, stderr 59 bytes, exit 1 |
| WSL Linux Python → same Windows `codex.exe` | No `-c` overrides | Same EOF, stdout 0 bytes, stderr 59 bytes, exit 1 |
| WSL Linux Python → Windows `cmd.exe /d /c exit 0` | No Codex involved | stdout 0 bytes; stderr 59 bytes; exit 1; fixed WSL vsock-timeout signature; about 10 seconds |
| WSL → Windows Python control | No Codex involved | Windows Python never starts; same fixed WSL vsock-timeout signature |

The native exact-configuration proof read 1,051 protocol bytes internally and
printed only boolean metadata. Its `exit_before_cleanup` was null: exit 1 after
that successful proof was the intentional child kill during cleanup, not a
startup failure. The extra Rust configuration keys were accepted sufficiently
for initialization and account read.

No Windows `cargo` or `rustc` was found on PATH, so a native Windows Rust binary
was not built here. The parent's actual Linux Rust integration remains unverified
past this host boundary. The previously reported successful model turn belongs
to the separate native Windows Python proof; it does not prove the Linux Rust
path or multi-turn resume.

## Reproduction

Diagnostic scripts live beside this report. Each uses only owned child
processes and public synthetic operations. Account reads set `refreshToken=false`.
They never open credential/token/config files and never print raw account replies,
environment values, arbitrary stderr, or other chat data.

Native exact-configuration account-only proof, from repository root:

```powershell
& 'C:\Python312\python.exe' '_dev/verification/router-memory-harness/probe_codex_host.py' rust
```

WSL exact-configuration account-only proof:

```powershell
wsl.exe -d ArchieOS-Operator-RTX3070-Test -- python3 /mnt/c/Users/Penta/.codex/worktrees/router-memory-harness/ArchieOS/_dev/verification/router-memory-harness/probe_codex_host.py rust
```

Independent WSL interop control:

```powershell
wsl.exe -d ArchieOS-Operator-RTX3070-Test -- python3 /mnt/c/Users/Penta/.codex/worktrees/router-memory-harness/ArchieOS/_dev/verification/router-memory-harness/probe_wsl_interop.py
```

The account script has a 20-second total deadline, 256 KiB frame limit, 2 MiB
protocol budget, and 65,537-byte stderr cap. The interop control has a 15-second
child deadline. No inference or GPU workload is performed.

## Supported next step

Preserve the current managed credential ownership and stdio protocol. Run the
runtime on the same native Windows host as the authenticated official client,
or repair the WSL host interop route before treating a Linux child-spawn test as
an authentication test. Re-run the exit-0 control first, then account-only, then
the explicitly authorized synthetic Rust completion/resume proof.

Do not compensate by exporting tokens or changing account identity parsing.
No configuration delta explains this failure: baseline, minimal, and exact
configurations fail at the same earlier boundary. A distribution restart may
restore interop but was not attempted: it would interrupt other agents and
their active workloads. Coordinate that host maintenance separately.

Official OpenAI documentation describes stdio JSONL and the initialization
handshake. It currently labels the TCP WebSocket transport experimental and
unsupported, so adding an ad hoc network bridge would not establish the
requested supported stdio integration. See
[Codex App Server](https://learn.chatgpt.com/docs/app-server).

Microsoft documents Linux-to-Windows executable launching and redirected pipes
as supported interop behavior. See
[Working across Windows and Linux file systems](https://learn.microsoft.com/en-us/windows/wsl/filesystems).
An open issue in Microsoft's WSL repository reports the same timeout class when
the init interop fallback fails, including cases with missing/stale
`WSL_INTEROP`; it is corroborating symptom evidence, not proof of this host's
internal root cause. See
[WSL issue 41283](https://github.com/microsoft/WSL/issues/41283).

## Bounded environment follow-up

A further read-only check was authorized to look for a native compiler outside
PATH and an existing alternate interop route. No host configuration, process,
environment, authentication, or product behavior was changed.

- `C:\Users\Penta\.cargo\bin` and `C:\Users\Penta\.rustup\toolchains` do not exist.
- Literal compiler paths under `C:\Rust\bin`, the legacy Rust Windows installer
  directories, Chocolatey shims, Scoop shims, and MSYS2 mingw64/ucrt64 contain no
  `rustc.exe`/`cargo.exe` at the checked locations.
- VS Installer discovery (`vswhere.exe`, requiring the x86/x64 VC tool component)
  finds `C:\Program Files (x86)\Microsoft Visual Studio\2019\BuildTools`.
  That supplies a native C/C++ toolchain, not a discovered Rust compiler or Cargo.
  Standard VS2022 BuildTools MSVC locations are absent.
- `/run/WSL` contains exactly `1_interop`, which is a Unix socket and not a
  symlink. No alternate session-specific socket is present.
- `/proc/sys/fs/binfmt_misc/WSLInterop` is present, enabled, and registers `/init`
  as its interpreter. The failure is therefore not an absent/disabled binfmt
  registration. `WSL_INTEROP` remains absent in the diagnostic process.

The sole existing interop socket is the init fallback already exercised by the
failing Windows exit-0 control. Its presence does not establish a healthy
Windows-side channel. There is no observed alternate live socket to select in a
child environment, and no native Windows Rust toolchain was found within the
authorized bounded locations. A complete native Rust managed proof consequently
requires an externally provided Windows Rust toolchain or coordinated recovery
of this distribution's interop route; neither has been performed here.
