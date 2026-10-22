# Npcap installed; real host LAN capture — 2026-10-13

Owner asked to finish installation. Same Nvidia repo, worktree and Draft PR39.
Prior user authorization retained; no additional scope approval requested.

Initial checks: non-elevated token, Npcap absent, signed installer available.
Computer Use runtime failed before initialization with kernel-assets path error3;
one reset/retry failed too. No alternative UI input/auth-dialog automation used.

Successful path: generated a private operational PowerShell helper, launched stock
Windows PowerShell with normal runas elevation. Helper entered installer phase;
official interactive Npcap1.89 installer exited0. Service npcap Running; TShark4.6.9
loads Npcap1.89/libpcap1.10.7. No silent free-installer switch, UAC bypass, security
setting change, firmware write or router login. Helper stays outside repo because
its paths are specific to this operator's temporary installation/diagnostic session.

First test selected Wi-Fi by label, captured0 frames. Windows PowerShell5 also
treated the native stderr summary as ErrorRecord under Stop, causing a false helper
failure. Changed helper to evaluate native exit code; no product code fix required.

Sentry itself identified current bindings/scope. Its active default LAN address was
on Ethernet6. Crossed that returned name with archie-netwatch interfaces, rather
than OS personal-device inventory or hardcoded private IP/MAC. Started current
archie-netwatch --seconds20 and concurrently ran archie-lan scan --seconds6 hybrid
to generate discovery traffic. Fresh capture from normal non-admin host session:
- exit0,565 parsed frames;
- 376 frames/89480bytes attributed across4 MAC records;
- 4 new-device notice alerts,0 confirmed hostnames;
- no error in capture log; no remaining netwatch/TShark/dumpcap process.

This proves host LAN capture, not four identified IoT devices, phone/printer ownership
or visibility of all traffic between other nodes. All private identifiers, DNS names,
logs and reports stay in TEMP/archie-router-audit-20261013. Current scan advertises
router and television/address aliases; aliases are not extra unique appliances.

Updated docs/SENTRY-ROUTER-AUDIT.md and docs/PENDIENTES.md to close the missing-driver
and host-capture tooling gap. Credentials/router capability probe, gateway capture,
universal IoT coverage and deployed Core producer remain explicit pending work.

CI on functional head fe83f38 passed: rust6m19s, native-contracts1m22s, run38358021836.
Local90 LAN tests/Clippy/rustfmt proof remains on that same functional head. This
follow-up changes documentation only; latest docs commit has independent CI status.
No merge or release/WSL image rebuild.
