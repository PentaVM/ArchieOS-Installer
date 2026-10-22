# Sentry autonomy ledger

Repo PentaVM/ArchieOS-Nvidia; branch codex/sentry-discovery, Draft PR39; main
8e0cf22, previous head 0ad5061. No GUI/WSL/release edits or authority replacement.
Owner explicitly requested a stronger autonomous Sentry and GitHub library research.
Direct authorization remains; no additional approval prompt.

Upstream comparison and reproducible commands: docs/SENTRY-AUTONOMY.md.
Implementation plan: docs/superpowers/plans/2026-10-13-sentry-autonomy.md.
Pinned mdns-sd 0.21.5 (MIT/Apache-2.0); reused workspace fs2 for cooperative locks.
BLE/MQTT/Thread/Zeek/Suricata remain surveyed follow-ups, not claimed integrations.

Implemented: bounded native/library/hybrid DNS-SD backend, scoped private IPv4/IPv6
conversion, observation history, scope reset, light/deep clocks, retry backoff,
private health/state, once mode, atomic per-file publication and locks. Inventory
v1 and Core/Policy/leases/adapters stay in place. History/cache-only entries never
refresh peer timestamps or live feeder events. Old power/app state is cleared;
current ARP cache metadata remains explicitly cached. A changed MAC drops old labels.

Review evidence: first directed review found two Important issues. A real old-binary
publication failure produced health failures=0 and last_success set despite output
being rejected; new CLI regression verifies failures=1 and no successful history.
Publication/validation errors now enter backoff. Success/deep is not persisted before
the consumer inventory is visible. Sidecars are not a multi-file transaction.

mdns-sd source inspection showed certain bind/join errors logged without monitor Error
events; IpAdd can also be emitted after a failed join on an already occupied interface.
No constructor/no-error/registration-only availability claims remain. Valid bounded
service resolutions establish backend reception; actual IPv6 transport coverage is
still explicitly unconfirmed because AAAA records may arrive via IPv4. Two regressions
exercise the silent/no-result and registration-only cases. Targeted re-review of those
two changes reported no Important/Critical findings.

A subsequent review found a missing baseline when a MAC first appeared after an earlier
live response without a MAC. Last neighbor MAC is now persisted separately from live
response time; the restart/change sequence has its own regression. No device owner or
hardware/model from OS device inventory is an input.

The owner wants light context protocols. Another real regression showed identical
PowerOn state at times100/130 differing only by history.last_response_unix. These
clocks now remain in history/health and cache-only annotations, rather than live
device facts. The Core integration regression traverses the actual controller,
Snapshot parser and diff function to prevent artificial state events each poll.
Directed review of the MAC and clock fixes reported no Important/Critical issues.

Windows debug builds exhausted disk during wider feed compilation (OS112/PDB LIMIT).
No failed build is counted as passing. Recovered space with Cargo's own cleanup of
this worktree's duplicate target and generated Core dev package artifacts only;
source, other worktrees, WSL images and private capture evidence were preserved.

Private black-box evidence: TEMP/archie-sentry-autonomy-20261013. The first hybrid pass
resolved eight service/address rows through mdns-sd and merged into seven address
records (five current service addresses, two cache-only candidates). IPv6 aliases of
Roku and Spotify are address claims, not two additional uniquely identified appliances.
TCP coverage completed 3795/3795 attempts. Health failures=0. Restart with identical
arguments/history preserved last_deep and emitted presence=null in diagnostics, while
light observations refreshed. No printer or phone ownership was established; no print
job was sent. Private IPs/MACs/full inventory are not pushed to GitHub.

Baseline CI at 0ad5061: rust and native-contracts passed. This does not certify the new
autonomy head. Current verification results are appended after final commands finish.
TShark is unavailable on the Windows host; real radios/hubs, IPv6 multicast transport
and deploying the new binary under Core remain explicit hardware/deployment checks.

Final local proof after all functional fixes:
- archie-lan full suite: 88 passed, one TShark test ignored (binary unavailable).
- Core discovery/history/feeder contract: 4 passed, including unchanged-state clocks.
- Sentry feed suite: 20 passed. Total relevant runnable proof: 112 tests.
- archie-lan clippy --all-targets --no-deps --locked -D warnings: passed.
- workspace rustfmt check and git diff --check: passed. The Unix-only symlink path
  test is not represented as executed on Windows; Linux CI must execute it.
- Fresh final binary on owner LAN: seven addresses, zero monitor failures, completed
  TCP coverage, no history clocks on live rows, two cache rows retaining neighbor MAC.
  Upstream IPv6 transport registration/delivery remains explicitly unconfirmed even
  while it yields scoped AAAA service claims. Temporary once-mode processes exited.

PR39 remains draft; CI for this new head is independent of the earlier green baseline.
The existing release/WSL image is not rebuilt or updated by this change.
