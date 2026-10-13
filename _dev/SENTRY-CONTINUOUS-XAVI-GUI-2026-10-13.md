# Continuous Sentry and Xavi GUI audit — 2026-10-13

Owner requested continuous capture and a check of Xavi's new GUI branch/PR. Work
remains exclusively in ArchieOS-Nvidia. GUI audit is read-only; no merge, GUI changes,
messages to Xavi or release/WSL changes are part of this follow-up.

## Host observation now running

`archie-netwatch live` without --seconds captures continuously through the NPF
interface selected by Sentry's current discovery scope/binding. Its owner-private
state is in LOCALAPPDATA/Archie/sentry/netwatch, with runtime identity metadata next
to it. Existing measured state/told are preserved; notices are not deliberately
reset on restarting the collector.

Also started the existing `archie-lan watch --autonomous`: light discovery30s,
deep TCP600s, hybrid native/mdns-sd,6s discovery window, existing bounded TCP budget,
private history/health/coverage in LOCALAPPDATA/Archie/sentry. No fixed home IP/MAC
or Windows personal-device inventory used. The two collectors only observe.

Both run hidden as normal user processes. Copied the verified functional binaries
to versioned LOCALAPPDATA/Archie/tools/sentry-fe83f38 so running images do not lock
the project's debug build outputs. Handoff verified the exact PID/start timestamp,
stopped only its dumpcap child, let the parent save on EOF, and restarted against
the same state. First identity comparison failed safely because ConvertFrom-Json
parsed the ISO timestamp as DateTime; compared UTC DateTimes rather than a string.

Verification: continuous state was13.4s old,6 observed MAC records and59517 attributed
frames; LAN watcher health had0 consecutive failures, successful fresh updates,
and7 address records (including aliases/cache, not seven identified appliances).
The IPv6 multicast transport limitation remains explicit. No raw identifiers or
packet/DNS contents committed. Capture and LAN inventory processes are alive.

Scope: this is a running host observation session, not a deployed Core-managed
service or Windows boot task. Runtime records explicitly mark core_managed=false
and autostart_installed=false. Linux already has the continuous netwatch systemd
unit; deployment/host-guest transport and feeder wiring still need the actual Core
runtime. No extra inference, actuation controller or Windows task scheduler added.

## GUI source and measured change

GitHub checked directly; fetched origin refs without touching another worktree.
Branch feat/professional-gui-quickshell-vault head a1a240efcc21ab3c0f38b183d1cdf44996f5dcc6.
Latest commit authored by ByXavi270 at2026-10-12 07:12:19 America/Santiago.
No new open PR for this branch. PR31 and PR34 are merged historical deliveries;
their fixed heads do not contain the later visual redesign even though PR31's
description was updated to describe it.

Four commits not in main:
- 357a1a3: Archie Mac visual redesign.
- bd2f4d6: second design handoff,8 surfaces/motion/81icons.
- d6630b9: smoke/honesty tests for samples and motion profiles.
- a1a240e: recorded owner's approval of four interaction changes (as reported by Xavi).

Comparison main...branch:161 changed files,+14132/-641 text lines.
- QML:45 changed files,+2192/-560 (includes UI tests).
- Reference HTML mockups:45 files,+9900/-0.
- Docs/tokens/other:20 files,+2040/-81.
- Images/screenshots/fonts:51 changed binary files, not represented as source lines.
- No new Rust/backend changes in these4 commits; existing bridge stays in place.
GitHub compare reports4 ahead/89 behind main. Requires integration against current
main rather than treating branch head as an up-to-date release.

Extracted immutable GUI snapshot under TEMP/archie-xavi-gui-a1a240e; inspected
committed desktop, Forge and Sentry screenshots, Theme/Main/Vault/Sentry QML, host
bridge, design decisions and test report. Removed only the redundant review ZIP;
extracted source/images retained for the owner to review.

## What the delivery contains and does not prove

Graphite/dark-only Mac-like shell: top menu bar, centred coloured dock, window
traffic-light controls, Inter/SpaceGrotesk bundled OFL fonts, cyan accent, amber
Archie indicators, work/personal chips and fallback opaque surfaces. Eight added
sample areas: window switcher, notifications/auth, Forge, IRIS/Cortex inspector,
settings, lock/start/install, workspace/orb placeholder and search. Four motion
profiles with actual QML transitions; frame-time/visual timing not measured.

Existing read-only bridge paths feed display/kernel, LAN inventory and long-job
status. New sample areas and Vault file operations remain Synthetic/Demo-driven.
Quickshell host layer still marked UNVERIFIED; full Linux/Wayland/NVIDIA deployment,
real blur, accessibility/performance and new voice UI integration not demonstrated.
Orbe/Lumenfield implementation is unchanged; OrbHole is a placeholder, not its
integration. Screenshots of printers/models/files are examples, not owner devices.

Xavi's report states21 BridgeModel checks and79 SurfacesTest checks passed offscreen
in Qt6.12/PySide6. We did not rerun those in this audit. Older baseline statements
in the audit/PR description predate newer main capabilities and must be re-audited,
not repeated as current missing functionality. Specific stale UI text:
SentryPanel.qml says Sentry does not scan and only lists SSDP/mDNS announcements,
while current Sentry supports WSD, bounded TCP, cache provenance and history.
Screenshot says Real bridge while showing example inventory; distinguish real
transport from actual input provenance. Visual integration is not universal E2E.

Updated pending design status so the new branch is not described as nonexistent.
No GUI implementation or branch merge performed. Functional Sentry head f7b7954 CI
rust/native-contracts passed before this documentation-only follow-up.
