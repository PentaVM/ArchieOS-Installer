# RFC-005 — Minimal ArchISO release profile

Status: accepted for ArchieOS v0.1 (M5)

## Context

The source tree contains legacy Python, tests, RFCs, and development artifacts that
must never be copied wholesale into ArchieOS. The ISO needs current Arch boot plumbing,
the two Linux release binaries, deterministic configuration, and an explicit build.

## Decision

- Track a conventional current ArchISO profile with BIOS/syslinux and
  UEFI/systemd-boot support.
- Compile `archie` and `archied` from the locked Rust workspace on the Arch build host.
- Copy the profile to an ephemeral staging directory and add only those two stripped
  release ELF binaries. Never commit host binaries or mutate the source overlay.
- Keep the live root headless and small: base/kernel/firmware, networking, Btrfs and
  installation tools, microcode, and VM guest support. No desktop, Python runtime,
  model weights, source tree, credentials, or debug tooling.
- Enable `archied`, systemd-networkd, resolved, and wait-online in the staged tree.
- Build only through `mkarchiso`; the upstream Arch ISO is a reference, not patched.

## Verification contract

Repository tests assert the profile/boot files, allowlisted packages, staged-binary
logic, service enablement, release exclusions, and absence of committed binaries. A
real M5 completion additionally requires `mkarchiso` on Arch Linux and an ISO checksum.
