# RFC-001 — Control endpoint lifecycle and single-node authority

Status: accepted for ArchieOS v0.1 (M1)

## Context

`archied` is the authority for one rooted Archie node. The original endpoint binding
removed a Unix socket or replaced the Windows endpoint publication unconditionally.
That recovered from a crash, but also allowed a second daemon to displace a live one.

## Decision

- One `archied` process may own a `StatePaths` root at a time.
- It holds an operating-system exclusive lock on `/run/archie/archied.lock` for the
  complete listener lifetime.
- Binding probes the published endpoint before and after lock acquisition. The first
  probe protects nodes still running a pre-lock daemon; the second closes the race.
- A lock file may survive a crash. Its contents are not authority; the OS lock is.
- Only the lock owner may replace a stale socket or endpoint publication.
- Shutdown drops and awaits the listener before removing runtime endpoint files.
- The control wire contract remains newline-delimited JSON and is unchanged.

## Consequences

Duplicate daemons fail closed with `AlreadyRunning`. Crash recovery stays automatic,
and different `--root` values remain independent for local multi-node simulation.
The `fs2` crate is part of the trusted control plane solely for portable file locking.

## Verification contract

Automated tests cover duplicate binding, stale locks, stale endpoint replacement,
missing-daemon errors, cleanup, and all read-only requests through a booted runtime and
real platform endpoint. A process-level acceptance test must additionally demonstrate
CLI success, duplicate refusal, clean shutdown, and stable identity after restart.
