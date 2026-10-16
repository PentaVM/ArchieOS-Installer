# Sentry: detection tools for the OS

What Sentry can add to `archie-netwatch` to know more about the owner's network, chosen
for this repository's rules: Rust or Go first, no Python in the runtime, licenses that can
ship in the image, and observation before any action. Each entry says what it would see,
what it costs and where it stands.

## What a passive sensor can see

On a switched LAN a host sees broadcast and multicast frames (ARP, DHCP, mDNS, SSDP,
LLDP, NBNS, IPv6 Router and Neighbor Advertisements) and its own flows. Everyone else's
unicast traffic reaches Sentry only when it runs on the gateway, or behind a mirror (SPAN)
port the owner configured. Every rule below is written so it still says something true
with only the broadcast view, and says more with a mirror.

## Implemented

| What | How | Licence |
|---|---|---|
| Capture and dissection | TShark subprocess, `dumpcap` holds the capture privilege, 768-byte snaplen, no payload fields | GPL-2 (separate process) |
| Hardware vendor | `oui-data` 0.2.3, IEEE registry built in; locally administered (randomized) MACs get no vendor | MIT |
| DHCP fingerprint | option 55 parameter request list, option 60 vendor class, option 53 message type | — |
| IP stack hints | initial TTL / hop limit (32/64/128/255), TCP window | — |
| TLS client fingerprint | JA4 (TLS client only), as TShark computes it | BSD-3 (JA4 client) |
| Names a device gives | DHCP host name, LLDP system name, NetBIOS name, HTTP User-Agent | — |
| `rogue-dhcp-server` | a second DHCP server answering after the learning window | — |
| `rogue-router-advertisement` | an IPv6 RA from a host that is not the router | — |

`archie-netwatch report` prints a `profiles` array: per device, the vendor, whether the
MAC is randomized, the hints with the evidence behind each one, and the raw fingerprints.
Hints are never a verdict: "the DHCP request list matches Windows" is reported as that, with
its basis.

## Next, in order

1. **huginn-net** (Rust, MIT/Apache-2.0). p0f-style TCP SYN signatures, HTTP header order
   and JA4 in one crate, without TShark. It would let a gateway build run the fingerprints
   in-process at line rate, and gives OS guesses with a quality score instead of the TTL
   heuristic. Contrast before adopting: run it and the current TShark path on the same
   pcaps and keep whichever agrees with the ground truth of the owner's own devices.
2. **In-process broadcast listener** (`pcap` + `etherparse` + `dhcproto` +
   `hickory-proto` + `oui-data`, all MIT/Apache). The broadcast view above needs no
   dissector: parsing ARP, DHCP, mDNS and RA directly removes the TShark start-up and the
   CSV round trip for the always-on part, and keeps TShark for full captures.
3. **Beacon scoring** in Rust, after RITA's method (regular intervals, constant sizes, long
   duration to one destination). RITA itself is GPL-3 and needs ClickHouse, so the method
   is reimplemented over the flows Sentry already keeps. This is the main signal for a
   compromised camera or plug calling home.
4. **Aya eBPF** (Rust, MIT/Apache; the kernel part GPL-2). On the gateway or the
   operator's own machine: per-process flow accounting with no packet copies, cheaper than
   pcap for "which program is talking". Needs a recent kernel and CAP_BPF.
5. **Zeek** as a subprocess (BSD-3), on a gateway or mirror port only. Its `conn`,
   `dns`, `ssl`, `x509`, `software` logs are a richer flow record than ours; Sentry would
   read them as it reads TShark today.
6. **Suricata** in IDS mode only (GPL-2, separate process) with the ET Open rules, for
   known-bad signatures. Never inline (IPS): a bad rule must not be able to drop the
   owner's traffic.

## Not adopted, and why

- **JA4+ beyond the TLS client fingerprint** (JA4S, JA4H, JA4X, JA4SSH…): FoxIO License
  1.1, not open source for commercial use. Only JA4 client is BSD-3.
- **Wi-Fi monitor mode and BLE scanning**: they see devices that are not the owner's.
  Under GDPR and the Spanish Penal Code (art. 197) recording third parties' identifiers
  needs a basis the owner usually does not have. If added later, only to match the
  owner's own enrolled devices, with nothing else kept.
- **Active scanning of anything but the owner's own devices** (nmap-style sweeps of the
  WAN, credential attempts, exploit checks): out of scope. Sentry asks a device it owns
  through the device's documented interface, after Policy and the owner allow it.

## How a tool is accepted

A tool joins Sentry when, on a recorded pcap of the owner's network and on the fixtures in
`core/crates/archie-lan/tests/fixtures/`, it finds something the current path misses or
finds the same thing cheaper, and its cost on the gaming budget in
[SENTRY-ROADMAP.md](SENTRY-ROADMAP.md) is measured, not assumed.
