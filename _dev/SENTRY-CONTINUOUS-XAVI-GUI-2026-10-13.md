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

## New user steering: PR40, native Orb

Owner pointed out PR40 during the audit. Queried GitHub directly and attached it:
design(orb): presets y escenas controlados por CLI, sin overlays, Draft/open,
XaviCottier, design/orb-presets-cli, c65543d78458a0106495861f23ca176bd1856aaa,
created2026-10-13T09:01:54Z. Separate from the large GUI branch above.9 files,
+221/-90, one commit at the observed head. No merge or edits of this branch.

Read CLI visual.rs, live voice tool changes, DisplayControl C++/header, Main.qml,
LumenField profileOverrides and the author's ledger. Adds archie visual scene/preset/
list and orb_preset through existing loopback channel, validated names. field has
no overrides and original defaults are unchanged by the diff. sphere/orb/neural
adjust existing renderer parameters (radial/category/hierarchy pull, scale,
membrane opacity, particles, node size, link budget/opacity, bloom/rotation).
No new shader code, halos/node shapes, styled links or live-brain emissions yet.

Compatibility change: removes voice orb_cinematic/orb_overlay, native protocol
cinematic/overlay and crown/atlas/dense presets. This is intentionally tested as
rejected, not backwards-compatible aliases. Existing capability preservation needs
review before merge; field-only preservation is not all-function preservation.

Rust and native-contracts CI green at c65543d. Inspected ci.yml: native job builds
cortex, sentry, display/visual and display/osm, not display/native (Qt). Therefore
green CI does not prove the modified C++/QML display compiles or renders. Author
reports CLI30 tests/check-voice; we did not rerun or render this PR. No actual new
Orb screenshot is present in the9-file delivery; GUI screenshots above are the
separate sample shell, not a render of these Orb presets.

## Latest published follow-up: PR41 and new PR40 commits

Fresh GitHub check after the owner's next status request found PR41 created at
2026-10-13T11:35:23Z (08:35 Santiago). GUI branch head9f98235 now has5 commits,
161 changed files,+14144/-644. Fifth commit fixes QML construction-time accesses,
negative repeater counts, Item.data alias shadowing and screenshot map; author
reports real-window testing of14 overlays,21 BridgeModel+79 Surfaces checks.
Its CI rust/native-contracts passes. PR description still contains warnings that
the latest commit explicitly addresses; use the commit/code as current evidence.

PR40 gained fb7ec85 (12:11:49Z/09:11 Santiago) and e460341 (12:37:53Z/09:37).
Head e46034161b9667a1ca2b696d61600b024b3610c4:3 commits,15 files,+281/-141.
New JSON presets embedded with Qt resources; overrides only recognised defaults.
Removes legacy comparison/toggleCompare/Antes as well as earlier retired controls.
Latest ledger reports successful native Windows compile/start (MSVC2022,Qt6.8.3,
CMake/Ninja), display answers preset sphere and Orb draws, but no nodes because
system observer7666 is absent. No node/preset visual validation with actual data,
no Linux/WSLg verification, no new halo/link/live-brain shaders. PR body still says
not compiled; it is stale relative to the new committed ledger. We did not reproduce
the native build or render. Native CI does not cover display/native.

Read failing CI log: Rust job stops in cargo fmt --check at visual.rs138/147; no
functional test failure established because those tests are not reached. Native
contracts passes. Source inspection only, no edits/merge of Xavi's implementation.
Attached PR41 to this task. Earlier no-new-PR statements describe the earlier query;
this new result supersedes them. No polling automation created.

## Owner steering: test the branches before merging, including non-Rust code

Exported exact PR40 e460341 native/visual/OSM sources and PR41 9f98235 shell into
isolated temporary snapshots. Did not edit Xavi/Claude's worktrees or merge.
Recovered 1960 MiB by deleting only regenerable PDB symbols from this audit
worktree's core/target/debug and deps; no source, executable, WSL image or shared
build removed. Used existing toolchains, no downloads or WSL configuration changes.

Native Linux build completed (GCC16.2.1, Qt6.11.2, CMake/Ninja). Visual19/19 and
OSM2/2 passed. Added external audit harness linked to those exact objects/resources:
82 preset checks passed, covering actual embedded JSON application/reset/defaults.
Native WSLg Wayland run produced a real PNG and exited0; seven accepted control
commands and five rejection cases passed. Renderer log explicitly says llvmpipe,
with Zink/EGL fallback warnings: no NVIDIA validation or performance claim. Observer
7666 absent, so this render did not validate live nodes. Port7668 was free before
starting, and the isolated process exited; existing producers were not replaced.

GUI stock Qt6.11.2 tests reported21+79 passes on Windows/Linux; Windows processes
exited0. Temporary WSL command connection failures/timeouts prevent treating its
exit statuses as clean proof. Windows normal-state real Qt offscreen window
cycled14 overlays and saved a desktop PNG with no QML warnings in that traversal.
Surfaces suite reproduced undefined QColor warnings at WindowSwitcher.qml35 in
both platforms: unknown-app key is empty in degraded; invisible Rectangle still
evaluates Theme.dk[info.key]. Assertions do not count engine warnings. Not fixed
in the owner's branch by this audit. Need strict warning checks before integration.

PR40 Rustfmt failure independently reproduced on a file copy. Retirement of voice
cinematic/overlay/Antes remains a compatibility blocker; capability registry/index
still reference removed tools and symbols. Main-to-PR40 is0 behind/3 ahead;
GUI is89 behind/5 ahead. Both merge-tree computations are conflict-free, not proof
of deployment. Read-only GUI bridge sources match current main. Preserving main
is mandatory when integrating this older GUI branch.

Reproducible harnesses, pinned SHAs, command recipes and remaining checks are in
_dev/verification/xavi/. No GUI implementation, release or WSL image modified.

Follow-up: the sample shell also rendered in WSLg/Wayland, cycled14 normal-state
overlays, saved PNG and exited0; no QML warnings, same llvmpipe renderer/fallback
warnings. This still does not test Quickshell. During the preservation check LAN
watch was alive, but passive netwatch had ended at09:40 local, before this audit
began. Its private log reported a completed capture (709438 frames), without a
failure reason. Restarted the existing pinned collector in the same continuous
mode/state directory/interface under the standing capture authorization; no data
reset, service installation or inference/authority change. Fresh private state
writes confirmed after restart. Durable Core-managed supervision remains pending.
