# Security and operator-control model

Status: v0.1 baseline

## Authority hierarchy

```text
authenticated operator
    -> policy decision
        -> scoped, expiring, revocable lease
            -> sandboxed worker or device adapter
                -> audited result
```

Models propose; they do not authorize. Modules receive only the capabilities and data
covered by their lease. Peer trust, cloud disclosure, filesystem scope, application
control and physical actuation remain separate decisions.

## Current enforcement

- `archied` runs as dedicated user/group `archie` with no Linux capabilities.
- systemd applies `NoNewPrivileges`, a strict system view, home protection, namespace,
  realtime, SUID/SGID and architecture restrictions compatible with hardware probing.
- Local CLI access is limited by Unix socket ownership and mode.
- One daemon owns a state root through an OS exclusive lock.
- Control and fabric messages have strict size bounds and parse failures do not panic.
- Configuration rejects unknown keys and unsafe values.
- Node identity uses persistent Ed25519 keys; peer announcements are signed.
- Receiver-owned trust prevents a peer from self-promoting.
- Routing privacy is a hard gate, not a score that cost or quality can outweigh.
- Policy defaults to no physical safety boundary and supports emergency pause.
- A physical-device lease is valid only for one prepared operation, one dispatch, a
  present operator and at most five minutes; broader scope is refused even for an
  authenticated human. The generic lease API rejects its one-operation scope without
  the prepared identifier, so an adapter cannot silently skip the binding check.
- A native device-adapter manifest cannot contain a network endpoint, credential, raw
  command, firmware image or executable path. Generated manifests are proposals only;
  activation requires a reviewed ISO-built module, fixed vocabulary, pairing and the
  existing one-operation lease. Firmware apply contracts require dry-run, receipt,
  inspect, stage and rollback declarations before a later dispatcher could consider it.
- The server-only inference harness stores an input digest in its plan and has no
  provider endpoint, credential or device inference placement. Paired devices can only
  be typed result targets.
- Durable audit writes can fail closed before future authority is granted.
- Release staging excludes source, VCS data, tests, screenshots, secrets, debug tools,
  Python and model weights.
- The native Qt Quick 3D client is a separate unprivileged account, can reach localhost
  only, and cannot read Archie state, keys or the control socket.
- The display projection can read the control socket but has no Internet egress. The
  external world fetcher has Internet egress but no Archie state/control access; only
  normalized typed records cross their loopback boundary.
- Missing or stale physical-world data is visibly degraded and produces no invented
  observations.
- `archie-cortexd` is a separate `archie-cortex` account with no Linux capabilities,
  no network namespace, AF_UNIX-only socket activation, bounded frames and a 384 MiB
  baseline envelope. Its production socket accepts only `archied`'s UID via
  `SO_PEERCRED`; a group mode alone is not treated as authority.
- Cortex reads policy-filtered immutable graph snapshots and cannot read the raw private
  vault, edit verified graph state, issue a lease, execute a task or promote a model.
  Sensitivity is an exact policy allow-list before context selection, never a model score.

## Permission behavior

The requested long-term behavior is **high operator control**, not unconditional
automation. The intended broker can remember explicit operator grants and handle
routine low-risk prompts, while broad, new, irreversible, credential-bearing, privacy-
sensitive or physical requests must still surface for approval.

The active v0.1 control protocol exposes no mutations, so it cannot yet approve app
permissions or dismiss operating-system prompts. Implementing that requires:

1. authenticated operator sessions;
2. resource-specific permission descriptors;
3. preview/dry-run where possible;
4. policy evaluation and durable audit-before-action;
5. expiring/revocable grants;
6. OS-specific brokers running outside the Rust trusted core;
7. a hard path for the operator to pause or revoke all automation.

## Trust states and data boundaries

- **Unverified:** visible but not schedulable for trusted work.
- **Pinned:** a valid key was seen through TOFU; identity continuity is known.
- **Trusted:** operator-owned promotion permits policy-selected work.
- **Revoked:** excluded even if signatures remain valid.

Signed discovery provides identity and message integrity. It does not prove that a
remote host is uncompromised or that advertised hardware exists. Sensitive placement
therefore requires both trust policy and, in a later milestone, stronger attestation.

## Known gaps before broad authority

The Cortex service receives a named traverse-only ACL on `/var/lib/archie` so it
can reach its own state and the explicitly shared graph. It does not join the
Core group or gain parent-directory listing rights. Private child permissions
remain authoritative. The `a+` directive appends a named ACL as documented by
[systemd](https://github.com/systemd/systemd/blob/main/man/tmpfiles.d.xml).
Cross-file packaging tests verify this configuration relationship; actual ACL and
service access still require a Linux boot check. Check both positive graph access
and negative Core database/control-socket access before calling isolation validated.

- Mutating control API and authenticated operator sessions.
- Secret store and provider credential isolation.
- Live scheduler-to-executor transport plus task-runner sandbox application. The
  executor, cgroup and Landlock negotiation contracts exist, but they are not yet an
  installed worker path.
- Durable lease store with restart-safe revocation.
- Encrypted cognitive-memory stores and retention/deletion policies.
- Trusted temporal graph ingestion and correction parser, model training/replay/shadow
  promotion and measured ONNX/TensorRT backends for the Analytical Cortex. The existing
  trajectory store requires an authoritative graph-update receipt and stores only
  hash-verified opaque references, but it is not a graph writer or a training service.
- Network encryption/session replay protection beyond signed discovery envelopes.
- Package/release signing and verified update/rollback.
- Device-specific paired agents, safety adapters, external physical interlocks and
  simulator E2E coverage before any router/camera/display transport is enabled.
- Independent security review and adversarial multi-node tests.

Until those gates close, ArchieOS must remain read-only toward applications and
physical devices by default.
