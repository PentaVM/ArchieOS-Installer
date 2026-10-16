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
| `dhcp-starvation` | 30+ unknown DHCP clients in a minute, by the DHCP client address (`chaddr`) | — |
| `beacon-suspect` and `beacons` | a clock-like rhythm of connections to one outside destination, scored as RITA does; told only for a destination taken up after the device was learned | — (method reimplemented) |

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
3. **Beacon scoring**: first part done (see the log below). Still to do: the data-size
   half of RITA's score (constant request sizes) and UDP contacts, which need per-flow
   byte counts the capture does not keep yet.
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

## Log of the improvement rounds

### 2026-10-16: beacon scoring

- **Compared.** RITA (Active Countermeasures; GPL-3, needs ClickHouse) scores the intervals
  between connections by Bowley skewness and median absolute deviation, plus connection
  count and data sizes. BAYWATCH (Hu et al., IBM, DSN 2016) finds periods with a
  spectral analysis (FFT) and then filters by how many hosts reach the destination. Flare
  and Zeek beacon scripts are in Python or need Zeek.
- **Chosen.** RITA's interval statistics, reimplemented (`archie-lan/src/beacon.rs`): they
  work on a short window (33 contacts) and cost a sort of 32 numbers per contact, where
  an FFT needs a long, evenly sampled series. From BAYWATCH, the idea that a rhythm alone
  is not news: the alert is kept for destinations a device took up after it was learned,
  and every other rhythm is only recorded on the device.
- **Tested.** A one-minute clock with ±6 s jitter scores 90 or more; exponential gaps
  (a person browsing) score under 70 in 20 seeded runs; bursts, 2 s polling and fewer than
  12 contacts are not scored; an audit test tells a new clock once and keeps a vendor's
  ten-minute keep-alive quiet; rhythms survive a save and a load.
- **Measured.** `archie-netwatch read` of 300,000 TCP SYNs from 50 devices to 1,000 outside
  addresses (the worst case: every frame a new connection out), 3 runs each, the
  process's own CPU without TShark: 2.13–2.25 s before, 2.38–2.49 s after, about 0.8 µs
  per connection. The first version swept the full contact map on every frame once it held
  4,096 pairs (6.4–7.0 s); pruning is now at most once a minute.
- **Not done.** A pcap fixture: the alert needs a device learned for a day, which a
  fixture of that length does not justify; the unit tests drive the same code path.
  Popularity filtering (BAYWATCH) waits for more than one sensor.

### 2026-10-16: DHCP starvation

- **Compared.** Switches stop it actively: DHCP snooping rate limits (Cisco's
  `ip dhcp snooping limit rate`, 15 packets/s per untrusted port by default) and port
  security caps on hardware addresses per port. Snort/Suricata have no stock rule for it;
  Zeek's `dhcp.log` lists the clients, and detection is left to the analyst counting them.
  A passive sensor cannot rate-limit, and a per-sender rate misses tools that also vary
  the Ethernet source (DHCPig does; Yersinia can).
- **Chosen.** Count distinct client addresses (`chaddr`, a new TShark field
  `dhcp.hw.mac_addr`) not known as devices over a sliding minute, the passive counterpart of
  the snooping limit. It sees both styles: one sender behind many client addresses (named
  as such in the alert), and a flood that varies both. Known devices renewing never count.
- **Tested.** Unit test: 29 new clients in a minute plus 50 renewals is quiet; 40 clients
  spread over two minutes is quiet; one sender with 200 client addresses in ten seconds is
  told once, again an hour later if it goes on. TShark end to end: the fixture now ends with
  40 discovers from one sender, told as `dhcp-starvation` with that sender.
- **Measured.** 60,000 discovers in a minute from made-up clients: one alert, the process's
  own CPU 0.31–0.34 s before and 0.36–0.38 s after (2 runs each; about 1 µs per
  discover). The 300,000-SYN capture is unchanged (2.46–2.50 s vs 2.46–2.58 s). The window
  keeps at most 4,096 messages.
- **Next.** The same flood shape with source addresses instead of DHCP (`macof`, filling a
  switch's address table) would make one `new-device` alert per made-up address up to the
  4,096-device cap; it should become one alert for the flood.

## How a tool is accepted

A tool joins Sentry when, on a recorded pcap of the owner's network and on the fixtures in
`core/crates/archie-lan/tests/fixtures/`, it finds something the current path misses or
finds the same thing cheaper, and its cost on the gaming budget in
[SENTRY-ROADMAP.md](SENTRY-ROADMAP.md) is measured, not assumed.
