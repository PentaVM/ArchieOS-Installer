# RFC-004 — Arch-native daemon service

Status: accepted for ArchieOS v0.1 (M4)

## Context

ArchieOS must boot into Archie without a desktop session or manual command. `archied`
needs durable state, an ephemeral control directory, readable configuration, structured
logs, restart behavior, and the least privilege compatible with hardware discovery and
LAN fabric traffic.

## Decision

- Run one `/usr/bin/archied` as the dedicated `archie` user and group.
- Use systemd `StateDirectory`, `RuntimeDirectory`, `LogsDirectory`, and
  `ConfigurationDirectory`; keep identity/database under `/var/lib/archie` and the
  socket/lock under `/run/archie`.
- Use `Type=simple`. The daemon does not claim `sd_notify` support it does not have.
- Emit JSON to stdout/stderr for journald. Operators use `archie ping` for readiness.
- Restart on unexpected failure, but do not loop on configuration exit status 78.
- Remove all Linux capabilities and enable compatible systemd hardening. Do not hide
  `/dev` or hardware/proc/sys views needed for accurate probing.
- Members of group `archie` may traverse `/run/archie` and use the 0660 control socket.
- Identity creation remains inside the deterministic runtime boot; no second first-boot
  authority is introduced.

## Verification contract

Repository tests parse the shipped TOML with the Rust configuration contract and assert
the service, sysusers, tmpfiles, and preset contain the required identity, directory,
restart, logging, and hardening directives. Linux validation additionally runs
`systemd-analyze verify` when a Linux/Arch environment is available.
