# Sentry discovery implementation ledger

Repository: PentaVM/ArchieOS-Nvidia, base main 8e0cf22.
Execution: direct, requested by owner; no device list, model, IP or manufacturer
from the earlier Windows inventory is an input to discovery or printer selection.

Baseline: archie-lan tests 42 + netwatch 4 passed; one TShark integration test
ignored on Windows because the binary is unavailable. Live native inventory:
three entries, no printer/mobile; WSL inventory empty, passive capture one entry.

Existing worktree reused on codex/sentry-discovery; Wanda PR38 remains separate.
Kimi's Sentry work is already in main via PR36; no duplicate control authority.

Plan: docs/superpowers/plans/2026-10-12-sentry-discovery.md.
Ruling: explicit authorization to proceed overrides skill approval prompts;
preserve written design, plan and test gates.

Implemented: multicast and unicast DNS-SD, explicit service and SRV/TXT/address
questions, correlated WSD/ONVIF observation, opt-in bounded TCP presence, Sentry's
own interface-scoped ARP-cache collection, Spotify read-only metadata and a coverage
sidecar. Inventory v1, existing adapters, Core policy, grants and journal remain.

Ruling from live evidence: serial per-host port probing exhausted 15 seconds after
65 of 253 hosts. Port-first scheduling now covers all hosts before deeper checks;
the deadline is configurable (default15, maximum120). A 75-second run completed
all 3795 planned probes on the selected subnet. Open ports did not identify new
hardware. The final inventory contains three service responders and three cache-only
candidates, without claiming those candidates are currently reachable or owned.

No printer was discovered, no print job was sent, no fixed printer/phone address
or model was used. Spotify's service address matches Sentry's local interface;
its getInfo reply failed the required success fields, so no mobile identity was
assigned. Roku also exposed a Spotify endpoint but returned an incomplete response.
The six-entry inventory, coverage and identifiers remain private in the owner's
temporary evidence directory, not in this repository.

Independent read-only review found four Important issues. Regressions demonstrated
the unassigned-interface success bug, false companion-link mobile classification,
cache-derived Core last_seen timestamp and cache-derived feeder DeviceSeen event.
All are fixed; targeted re-review reported no Important/Critical findings.
Actual multicast UDP reception and independent-source recovery are now exercised.

Verified locally after fixes:
- archie-lan: 68 tests passed; one TShark integration test ignored because TShark is
  unavailable to Windows Sentry. No claim of host packet capture validation.
- Core discovery contract: 2 passed; existing Core unify tests: 7 passed.
- Sentry feed integration: 20 passed; no cached candidate enters live events.
- archie-lan clippy --all-targets --no-deps -D warnings: passed.
- Native black-box rerun of the rebuilt collector: six entries, no printer.

Broader validation finished on 2026-10-13 (America/Santiago client date):
`cargo test -p archie-core -p archie-sentry --locked --jobs 1` passed, including all
126 Core unit tests, the new writer/parser/freshness contract and all runnable
Sentry integration suites. 379 tests passed; 27 tests declared ignored were not run
and are not represented as passed (PostgreSQL/explicit external fixtures).
`cargo fmt --all -- --check` passed. Total runnable proof with archie-lan: 447 tests.
PR CI is separate and remains pending until its actual result is read.
Real bulbs/hubs/IPv6-only nodes, passive Windows capture and physical printing
remain hardware/deployment validations, not solved by parser fixtures.
