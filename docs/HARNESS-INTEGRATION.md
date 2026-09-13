# Harness integration

Archie owns a small adapter boundary and can use HarnessRouter Community Edition's
Unified Harness Protocol (UHP). The client calls the loopback
`/api/harness/v1/responses` endpoint, supplies `metadata.harness_id`, bounds responses,
disables redirects, supports cancellation, and reports timeout and HTTP failures.
HarnessRouter remains replaceable.

Codex CLI, Claude Code and Hermes are first-run dependencies. ArchieOS does not
redistribute them. `archie harness login codex` delegates to `codex login`; Claude opens
its official interactive CLI login, and Hermes delegates to `hermes setup`. Archie never
reads, copies or converts their OAuth/session tokens.

```bash
archie harness list
archie harness add codex
archie harness login codex
archie harness test codex
```

The process probe uses an argv vector and no shell. Private context may reach a local UHP
worker only after the normal capsule and model/harness routing gates. `Secret` is rejected
before a TCP connection is opened.

HarnessRouter CE is not embedded in the image: its container and proprietary CLI
dependencies add substantial size and require accepting upstream terms. Operators can
install it at first run and point the UHP adapter at its loopback listener.

