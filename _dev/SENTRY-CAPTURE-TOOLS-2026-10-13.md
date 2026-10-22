# Sentry capture tooling follow-up — 2026-10-13

Owner asked to install Npcap, continue, or find a better GitHub alternative. Same
Nvidia worktree/PR39; no changes to other agents' worktrees, Core authority or release.

Installation diagnosis: verified official Npcap1.89 exists and Authenticode is valid.
Windows process token is not elevated. Start-Process runas gives a path launch error;
direct ProcessStartInfo/UseShellExecute=false reveals native740 (requires elevation).
Appinfo is running. Sent normal runas request through Shell.Application; service,
process and TShark checks have not shown installation success. Npcap remains absent.
PktMon is present but `pktmon status` returns access denied. No UAC/security changes.
Removed only the two cabinets and partial duplicate MSI created for this task after
verifying each resolved path within LOCALAPPDATA/Archie/tools. Installed binaries,
the full verified MSI, captures, source and other worktrees were preserved.

Compared upstream Npcap, WinDivert, PcapPlusPlus, NDISAPI, Microsoft PktMon/etl2pcapng.
Decision: keep existing TShark/libpcap backend. WinDivert is useful for host traffic
control, needs admin and offers IP/flow/socket observations rather than the same
Ethernet/ARP evidence. PcapPlusPlus is a multiplatform C++ wrapper over capture
engines, not a way to remove the driver's requirements. NDISAPI entails another
driver/backend. PktMon is a possible Windows diagnostic fallback but not available
to this non-elevated process. Sources/comparison in docs/SENTRY-ROUTER-AUDIT.md.

Bounded continuation: add `archie-netwatch live --seconds 1..86400`, delegated to
TShark -a duration:N. Normal EOF saves state and told even when idle. No flag keeps
continuous capture. Core still owns process/startup limits; no new authority layer.
Unknown/incomplete live options and invalid durations must not silently run forever.

TDD: tests/netwatch_cli.rs failed against previous code: --seconds0 tried to launch
TShark and returned1, creating state, instead of usage2 before any subprocess/state.
After initial fix, reviewer identified --learn/--gateway consuming --seconds as a
missing value. Added regression reproduced failure; take now leaves option tokens
for validation. Twelve invalid/incomplete/mistyped combinations pass. Directed
re-review reported no Important/Critical issues outstanding (static review).

Real Windows proof with current TShark4.6.9: extcapudpdump idle capture, --seconds2,
exit0 after4.65s including startup/shutdown,0frames, state.json/told.json saved.
This is an idle termination test, not owner LAN physical visibility. No driver claim.
The installed Linux Builder binary/release is not overwritten by this local change.

Final local commands after the parser fix:
- cargo test -p archie-lan --locked -- --include-ignored:90 passed,0failed,0ignored,
  including real TShark synthetic PCAP test. Unix-only checks remain for Linux CI.
- cargo clippy -p archie-lan --all-targets --no-deps --locked -- -D warnings:pass.
- rustfmt and diff checks recorded after final documentation updates.

Prior CI on8f13837:rust and native-contracts both passed. New functional commit
requires independent CI; not represented as covered by that earlier head.
