# Sentry router audit — 2026-10-13

Request: research authorized router security/control and missing tools, executing
diagnostics as ArchieOS. Owner says no credential/key exists in Vault. Preserve
Kimi's router adapter, Core/Policy/leases and Claude/Xavi's GUI/WSL ownership.

Audited router adapter, IGD/probe/SSH/client code, Core config/controller wiring,
LAN UPnP description parsing, netwatch capture and real TShark integration fixture.
Existing SSH/IGD implementation is retained. Gaps are documented rather than replaced:
router CLI probe, IGD control URL discovery, real authentication and physical capture.
Additional integration finding: RouterBinding::to_router_config marks SSH/IGD from
configuration; operator_session passes these flags without calling probe_router.
Radio tools stay empty (WifiSet Unsupported). Configured is not observed; wiring
the real capability probe in that cycle is another explicit pending item.

Linux proof: ArchieOS-Builder is Arch; TShark 4.7.4 was present. Installed Nmap
7.991-1 and tcpdump 4.99.7-1 through pacman. WSL interop/automount remain disabled;
private input/script/report were transferred through stdin/stdout. No image build.

Selected the sole current SSDP router from Sentry's existing LAN inventory. Nmap
connect/version-light only TCP22/80/443, max-rate5, retries1, host timeout45s with
external60s bound. Exit0, 30.36s, all3 open; SSH Dropbear2019.78. Other protocol
labels uncertain. No firmware version, login, remote shell or configuration write.

Actual installed archie-netwatch/TShark captured eth0 concurrently: exit0,208 frames
processed,206 assigned across2 virtual MAC records,2 new-device notices. This is WSL
transit visibility, not full LAN or two IoT devices. Temporary capture exited after
SIGINT to its TShark child. Private evidence: TEMP/archie-router-audit-20261013 and
Builder /tmp/archie-router-audit-20261013. No private IP/MAC/report pushed.

Windows winget downloaded/verified Wireshark4.6.9 MSI, but system installation could
not complete elevation and was interrupted. Partial direct-download duplicate is not
used. Extracted CLI/application dependencies with WindowsInstaller read-only tables
and 7Zip from the full verified cached MSI into LOCALAPPDATA/Archie/tools/wireshark-4.6.9;
no system directories/driver writes. Added this path to user PATH. MSI SHA256:
1975dfc96581a9b674fda3f41ac1585963b4ba52ea9459f3b6d9cedfac3a7217; Authenticode valid
Wireshark Foundation. Version and extcap/sshdump checks pass. Live physical interfaces
unavailable; TShark reports missing Npcap.

Downloaded Npcap1.89 from official distribution. Authenticode valid Nmap Software LLC;
SHA2568AED85E900D783D1308506E919587D3E540451947AF8A82F2D04F819E44305CC.
Launch failed in this Windows tool process even with an existing explicit working
directory; no installer success/driver availability claimed. Driver needs interactive
installer/elevation; no silent OEM switch misuse or repackaging in ArchieOS.

Fresh current-source proof (cargo, not merely old cached executable):
`cargo test -p archie-lan --test tshark_e2e --locked -- --ignored --nocapture` with
user-local TShark on PATH:1 pass,0fail,0ignored; runtime0.90s. Synthetic PCAP exercises
real dissector fields and spoof/tunnel/sweep alerts, not hardware threat validation.
This closes the earlier Windows ignored-test tooling gap. Existing112 relevant tests
are not rerun for documentation-only follow-up. CI rust/native-contracts passed on
7e2e748, actions run38350447438; new documentation commit has independent CI status.

Primary research: Wireshark/Npcap/sshdump, Nmap, OPNsense API, OpenWrt upstream,
official CVEProject record. CVE-2023-33381 requires authentication and exact described
firmwareAR_g5.8_110WVN0b7_2; actual firmware unknown, no exploitation performed.

Output: docs/SENTRY-ROUTER-AUDIT.md plus updated pending-tool status. No production
controller changes, merge, release publication or WSL image rebuild in this follow-up.
