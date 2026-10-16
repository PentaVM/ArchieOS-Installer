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
| `rogue-dns-server` | a new LAN device answering DNS after learning (not the router, not a learned Pi-hole) | — |
| `arp-sweep` | one device asking ARP for 50+ addresses in a minute (a LAN scan) | — |
| `icmp-redirect` | an ICMP/ICMPv6 Redirect from anyone but the router | — |
| `retired-tls` and `tls_versions` | a device whose TLS client stops at 1.1 or older, read from its JA4 | — |
| `rogue-dhcpv6-server` | a device other than the router answering DHCPv6 (mitm6) | — |
| `name-poisoning` | a device answering LLMNR/NetBIOS for 3+ names, or for `wpad` (Responder) | — |
| `mac-flood` | 20+ unknown hardware addresses sending in a minute; the flood is kept out of the inventory | — |
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
3. **Beacon scoring**: intervals and sizes done (see the log below). Still to do: UDP
   contacts (QUIC, DNS-over-UDP check-ins), which have no SYN to mark a contact.
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
- **Next.** The same flood shape with source addresses instead of DHCP (`macof`): done in
  the next round.

### 2026-10-16: switch table (MAC) flood

- **Compared.** Switches defend with port security (a cap on hardware addresses per port)
  and alarm on their own table filling up, which a home switch rarely reports. arpwatch
  reports each new station ("new station" mails), which is exactly the storm this replaces;
  Zeek and Suricata have no stock detection for it.
- **Chosen.** The same sliding minute as `dhcp-starvation`, now one `Burst` type for both:
  distinct hardware addresses not known as devices, from every frame, before any IP
  filter (macof's made-up IP addresses are mostly outside the LAN, so the IP-based device
  path never saw them). Nothing counts during the first minute or the learning time, when
  every device is new. During a flood unknown addresses are not made devices.
- **Before this.** A macof capture of 100,000 frames, read after a minute of quiet, gave
  399 `new-device` alerts and 399 made-up devices in the state (the 0.4 % of its random IP
  addresses that fall in private ranges); with a longer flood, up to the 4,096-device cap,
  after which real newcomers are not recorded.
- **Tested.** Unit test: 40 devices arriving in the first minute are quiet; two guests'
  devices later are two `new-device` alerts; a thousand made-up senders in a second are one
  `mac-flood`, fewer than 20 `new-device`, and known devices are still updated. TShark end
  to end: the fixture now ends with 60 macof-style frames, told as `mac-flood`.
- **Measured.** The same 100,000-frame capture: 400 alerts and 399 devices before, 3 alerts
  (one `mac-flood`) and 1 device after; own CPU 0.63–0.64 s before, 0.66–0.67 s after. The
  300,000-SYN capture: 2.52–2.59 s before, 2.32–2.54 s after (noise).

### 2026-10-16: LLMNR and NetBIOS poisoning (Responder)

- **Compared.** Respounder (Go, Rapid7) and similar canaries ask the link for a made-up name
  and flag whoever answers: certain, but active (Sentry would send queries), so it is left
  for a Policy-approved mode. Conveigh/Inveigh and Zeek scripts watch answers passively;
  Suricata has ET rules for specific Responder versions.
- **Chosen.** Passive: count the distinct names each device answers for over LLMNR (a DNS
  response from UDP 5355) and NetBIOS (an NBNS response; new TShark field
  `nbns.flags.response`). Three names, or `wpad` alone, is told once per device. From the
  canaries, the idea that answering for a name that is not yours is the whole signal.
- **Tested.** Unit tests: LLMNR and NBNS answers are read, a question is not an answer; a
  Windows machine answering ten times for its own name is quiet; Responder answering four
  names is one alert naming them; `wpad` alone is told. TShark end to end: the fixture now
  has a Windows machine answering for itself (quiet) and a Responder answering LLMNR for
  two names and NetBIOS for WPAD, told as `name-poisoning`.
- **Measured.** The 300,000-SYN capture, own CPU: 2.39–2.53 s before, 2.41–2.43 s after
  (noise); the only per-frame cost is one more TShark column.
- **Limit.** Answers are unicast; on a switched network only those to the sensor's own
  machine are seen. The bait mode above would cover the rest.

### 2026-10-16: rogue DHCPv6 (mitm6)

- **Compared.** Fox-IT, who wrote mitm6, recommend blocking DHCPv6 at the host firewall
  or using DHCPv6 Guard / RA Guard on managed switches (RFC 6105 for RA Guard; DHCPv6-Shield,
  RFC 7610): both active, both need equipment or Windows policy the home rarely has.
  Zeek logs DHCPv6 but has no stock alert; Suricata ET has signatures for some mitm6
  versions only.
- **Chosen.** The passive counterpart of DHCPv6-Shield, written like the existing rogue
  router advertisement rule: a DHCPv6 advertise or reply (new TShark field
  `dhcpv6.msgtype`, from UDP 547) from a device that is not the IPv4 router's owner is
  high; a new server once others are known is a warning. Servers are kept in the state.
- **Tested.** Unit test: the router serving DHCPv6 while learning and after is quiet; mitm6
  on a laptop after learning is told once, high; the state keeps both and loads. Capture
  test: an advertise from port 547 is an answer, from 546 it is not. TShark end to end: the
  fixture now has mitm6 answering the Windows client's solicit with itself as DNS server.
- **Measured.** The 300,000-SYN capture, own CPU: 2.48–2.49 s before, 2.47 s after.
- **Proposal, not done.** Blocking DHCPv6 from anything but the router is a router or
  switch change: a proposal through Policy with the owner's approval.

### 2026-10-16: retired TLS (cameras and appliances left behind)

- **Compared.** Zeek's `SSL::Old_Version` notice (policy `protocols/ssl/weak-keys`) and
  Suricata's `tls.version` keyword both read the version from the handshake; both need
  their own engine. The JA4 Sentry already keeps carries the highest version the client
  offered in its second and third characters, the same fact.
- **Chosen.** No new capture field: `fingerprint::tls_version` reads it from JA4, and a
  client offering nothing newer than TLS 1.1 (retired by RFC 8996) is a `retired-tls`
  notice once per device, naming the server it called; every profile lists
  `tls_versions`. A notice, not a warning: it is hygiene to fix on a device the owner has,
  not an attack. It feeds the roadmap's camera stage (which devices need firmware first).
- **Tested.** Fingerprint test: TLS 1.3 over TCP and QUIC, TLS 1.0, SSL 3.0 and garbage
  read correctly; a device with 1.0 and 1.3 clients lists both. Audit test: a TLS 1.3
  client is quiet, a camera offering TLS 1.0 three times is one notice naming
  `cam.example.net`. TShark end to end: the fixture now has a camera's TLS 1.0 ClientHello,
  which TShark fingerprints as `t10d030100_…`, told and listed.
- **Measured.** Not on the capture path: one string match on frames that already carry a
  JA4, nothing new asked of TShark.
- **Not done.** The server side (a device's own web interface offering only old TLS) needs
  the ServerHello version, a new field; it waits for the camera stage.

### 2026-10-16: ICMP redirects

- **Compared.** Hosts can refuse redirects (`net.ipv4.conf.*.accept_redirects=0` on
  Linux; `EnableICMPRedirect=0` on Windows), which protects one machine but says nothing.
  Snort's classic rules alert on every redirect (GPL rule "ICMP redirect host"), noisy
  where the router sends them; Zeek raises an `icmp_redirect` event without a verdict.
- **Chosen.** Alert on who sends it: the router (the ARP owner of the gateway, IPv4 or
  IPv6) is allowed; anyone else is high, once per sender; while the router is not known it
  is a warning under its own key, so the high one still comes once the router is learned.
  New TShark field `icmp.type`; ICMPv6 type 137 was already read.
- **Tested.** Capture test: IPv4 type 5 and ICMPv6 type 137 are redirects, an echo is
  not. Audit test: unknown router gives a warning, then high once the router is known; the
  router's own redirect is quiet; repeated redirects from one sender are one alert. TShark
  end to end: the rogue in the fixture now redirects the Windows client's DNS through
  itself, told as `icmp-redirect`.
- **Measured.** The 300,000-SYN capture, own CPU: 2.57 s before (one outlier run at
  3.41 s), 2.44–2.58 s after: one more TShark column, no measurable cost.

### 2026-10-16: ARP sweeps

- **Compared.** arpwatch keeps who-is-who but does not count questions; Snort/Suricata
  see no ARP (they start at IP); Zeek needs its `arp` analyzer script, not loaded by
  default. Sentry's `host-sweep` counted TCP openings only, which a switched network hides,
  while an ARP sweep is the first thing a LAN scan does and is broadcast.
- **Chosen.** Count the distinct addresses each device asks ARP about over a sliding
  minute (the same `Burst` window as the DHCP and flood rules; new TShark fields
  `arp.opcode` and `arp.dst.proto_ipv4`; gratuitous announcements excluded). 50 in a minute
  is a warning, once an hour per device: a scan, not yet an attack, and the owner's own
  network apps do it too.
- **Tested.** Capture test: a request is read, a gratuitous one is not. Audit test: a
  laptop asking for the router and a printer 200 times is quiet; 60 addresses at one every
  1.5 s is quiet; `nmap -sn` over a /24 in two seconds is one alert. TShark end to end: the
  fixture has 60 ARP requests in 1.2 s, told with the scanner's address.
- **Measured.** 100,000 ARP requests from one scanner: one alert; own CPU 0.55–0.60 s before,
  0.76–0.82 s after (about 2 µs a request, the window's strings). The 300,000-SYN capture
  is within noise (2.38–2.73 s before, 2.65–2.72 s after).

### 2026-10-16: rogue DNS servers on the LAN

- **Compared.** Suricata/Snort have no generic rule for "a new DNS server"; Zeek's
  `dns.log` lists responders and leaves the baseline to the analyst. Home tools (Fing,
  routers' own security features) do not look at who answers DNS at all. The attack it
  follows is usually a rogue DHCP server handing itself out as DNS, or a dns.spoof module
  answering first.
- **Chosen.** The same learn-then-tell shape as the DHCP and router rules: every LAN device
  that answers DNS (from port 53, flag already read, no new TShark field) to another LAN
  device is recorded; the router and anything seen while learning (a Pi-hole, a NAS) are
  quiet; a new one afterwards is a warning naming the ones before it.
- **Tested.** Capture test: a DNS answer from 53 is read, an mDNS answer is not. Audit test:
  a Pi-hole while learning and the router afterwards are quiet; a laptop answering three
  times is one alert naming the Pi-hole; an answer going to the internet is not counted.
  TShark end to end: the fixture's rogue answers the Windows client's question for
  `bank.example` with its own address, told as `rogue-dns-server`.
- **Measured.** Not on the capture path: no new TShark field, one check on frames that
  already carry a DNS flag.

### 2026-10-16: beacon sizes (RITA's second half)

- **Compared.** RITA scores data sizes the same way as intervals (Bowley skewness, MAD
  against the median) plus a "smallness" term; Flare and AC-Hunter weigh size regularity
  too. Per-flow byte counts would need a new flow table; Sentry already keeps, per device,
  the running bytes sent to each outside destination (`sent_to`).
- **Chosen.** The size of a contact is the difference of that running total between one
  contact and the next: no new state on the hot path, only one number per contact. Sizes
  get the same two terms as intervals, and the score is the mean of both halves (the
  intervals alone while fewer than 11 sizes are known, or after a restart sends the total
  backwards). "Smallness" is left out: a large constant upload is as clock-like as a small
  one. The median size is kept on the device (`beacons[…].bytes`) and named in the alert.
- **Tested.** A one-minute clock sending about 420 bytes each time scores 87 (still told);
  the same clock with exponential sizes scores under 85 (not told); intervals alone as
  before; a total that goes backwards starts the sizes again. The audit test's new clock
  sends the same each time and is still told once.
- **Measured.** The 300,000-SYN capture, own CPU: 2.44–2.53 s before, 2.46–2.52 s after.

## How a tool is accepted

A tool joins Sentry when, on a recorded pcap of the owner's network and on the fixtures in
`core/crates/archie-lan/tests/fixtures/`, it finds something the current path misses or
finds the same thing cheaper, and its cost on the gaming budget in
[SENTRY-ROADMAP.md](SENTRY-ROADMAP.md) is measured, not assumed.
