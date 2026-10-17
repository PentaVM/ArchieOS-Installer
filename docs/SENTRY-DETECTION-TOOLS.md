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
| `lease-conflict` | an ARP claim of an address the router leased to another device | — |
| `rogue-dns-server` | a new LAN device answering DNS after learning (not the router, not a learned Pi-hole) | — |
| `arp-sweep` | one device asking ARP for 50+ addresses in a minute (a LAN scan) | — |
| `icmp-redirect` | an ICMP/ICMPv6 Redirect from anyone but the router | — |
| `retired-tls` and `tls_versions` | a device whose TLS client stops at 1.1 or older, read from its JA4 | — |
| `rogue-dhcpv6-server` | a device other than the router answering DHCPv6 (mitm6) | — |
| `name-poisoning` | a device answering LLMNR/NetBIOS for 3+ names, or for `wpad` (Responder) | — |
| `login-burst` | 30+ connections from one device to one host's FTP, SSH, Telnet (23/2323), RDP or VNC port within a minute | — |
| `cleartext-password` | a client sends FTP `PASS`, POP3 `PASS`/`APOP`/`AUTH`, IMAP `LOGIN`/`AUTHENTICATE` or SMTP `AUTH` unencrypted; high when the server is outside. Only the command word is read | — |
| `dga-suspect` | 25+ different random-looking names answered NXDOMAIN to one device in a minute (a domain generation algorithm) | — |
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
3. **Beacon scoring**: intervals, sizes, UDP contacts and destinations named by DNS done
   (see the log below).
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

### 2026-10-16: the router's lease table (roadmap stage 2, first step)

- **Compared.** OpenWrt's `ubus call luci-rpc getDHCPLeases` (JSON, needs a session login),
  SNMP's `ipNetToMediaTable` (ARP cache, needs SNMP enabled, no names), UPnP IGD (no lease
  table at all), ISC dhcpd's `dhcpd.leases` (rare on home routers), and dnsmasq's lease
  file, written by OpenWrt, Pi-hole, DD-WRT, Tomato and Asuswrt-Merlin.
- **Chosen.** The dnsmasq file first: one plain format covers most home routers, and
  reading a file the owner copied needs no credential and changes nothing on the router.
  The API readers come later, through Policy, when Sentry is given a credential.
- **Done.** `archie_lan::leases` parses it (IPv4 only, hardware address and host name
  validated, at most 4,096 lines, files over 1 MB refused); `Audit::leased` adds the device,
  its address, the router's name for it and `leased_until_s`, without counting a lease as a
  sighting; `archie-netwatch leases FILE --state-dir DIR` applies a file. A device only the
  router knows is a `new-device` after learning, sharing the told key with the wire.
- **Tested.** Parser: three IPv4 leases read; broadcast address, bad IP, an unsafe name,
  the `duid` line, an IPv6 lease and garbage skipped. Audit: a lease for a device already
  seen adds its name without moving `last_seen_ms`; while learning a new one is quiet;
  afterwards it is told once, and the wire does not tell it again. CLI end to end on a
  fixture file: three devices kept with names and lease ends, a second read is quiet, a
  2 MB file is refused before anything is written.
- **Measured.** Not on the capture path.

### 2026-10-16: more lease formats

- **Compared.** Which routers the owner is likely to run beyond dnsmasq: pfSense and
  OPNsense (ISC dhcpd; OPNsense also offers Kea and dnsmasq), and OpenWrt reached through
  LuCI's RPC (`luci-rpc getDHCPLeases`, JSON) rather than the file. Kea's CSV lease file
  and SNMP remain for later.
- **Chosen.** Read all three by shape: JSON if it starts with `{`, ISC if a line starts
  with `lease `, dnsmasq otherwise. ISC keeps only active bindings and the last block for
  each address (dhcpd appends); its `ends` dates are UTC and converted with Hinnant's
  days-from-civil; OpenWrt's relative `expires` is added to the time the file is read.
- **Tested.** ISC: two blocks for one address keep the later (ends 2026-10-16 12:00:00
  UTC, checked against Python's `calendar.timegm`), a free binding is skipped, `ends never`
  is no end. JSON: relative expiry, `false` as no end, a bad MAC and the IPv6 list skipped,
  broken JSON gives nothing. CLI end to end on a pfSense-style fixture.
- **Measured.** Not on the capture path.

### 2026-10-16: lease conflicts (the router as a witness)

- **Compared.** arpwatch and `address-moved` see an address change hands, but only between
  two claimants Sentry has heard; a switch's DHCP snooping binding table (Cisco Dynamic ARP
  Inspection) drops ARP that disagrees with the leases, which a home switch cannot do.
  Zeek's `dhcp.log` and `arp` analyzer would need joining by hand.
- **Chosen.** The passive half of Dynamic ARP Inspection: keep the router's lease table
  (`leased_to`: address → holder and lease end) and tell an ARP claim that disagrees with a
  lease still holding. It works even when the real holder has not spoken since Sentry
  started, and the other way round: a lease read after the wire showed someone else on the
  address is checked too. A warning, not high: a static address set inside the DHCP range
  looks the same and is far more common.
- **Tested.** The holder claiming its own address is quiet; another device claiming it is
  one alert naming the holder; after the lease's end the address may move freely; a lease
  read after a conflicting claim is told; the lease table survives a save and a load.
- **Measured.** One map lookup per IPv4 ARP claim; nothing else on the capture path.

### 2026-10-16: reading the router over SSH (roadmap stage 2, second step)

- **Compared.** OpenWrt's `ubus`/LuCI RPC over HTTP (needs `rpcd` ACLs and a session
  password kept by Sentry), SNMP (`ipNetToMediaTable`; v2c sends a community string in the
  clear, v3 is rarely on in home routers), UPnP IGD (no lease or ARP table at all) and SSH
  with a key pinned to one command (`authorized_keys` `command=`), the way backup tools
  such as rsnapshot are given narrow access.
- **Chosen.** SSH with a pinned command: it works on OpenWrt, Asuswrt-Merlin, pfSense,
  OPNsense and any Linux router, Sentry keeps no password (the owner types it once, to
  `ssh`), and the router itself enforces that the key only reads. The host key is pinned on
  first contact. Neighbours read from the router go through the same address checks as ARP
  from the wire (`Audit::router_saw`).
- **Tested.** Kind detection from SSH banners and web titles; the key line and that no read
  command writes, deletes, reboots or kills; the `ssh` arguments (key only, no agent, pinned
  known hosts, no `BatchMode` only for the interactive install); parsing of the combined
  output (dnsmasq and ISC leases, Linux `/proc/net/arp` with incomplete entries skipped, BSD
  `arp -an`); a device only the router's neighbour table shows is told as `new-device` after
  learning, and that table showing another device on the router's address is `address-moved`
  (high).
- **Not tested here.** No real router: this cloud container is not on the owner's network.
  Enrolment and the periodic read must be tried on the owner's machine against their router
  before they are called working.
- **Measured.** Nothing on the capture path: the read runs on its own thread every 5 minutes
  and the capture loop only checks a channel.

### 2026-10-16: beacons over UDP (QUIC, DNS check-ins)

- **Compared.** RITA scores UDP from Zeek's `conn.log`, where a UDP "connection" ends after
  `udp_inactivity_timeout` (1 min); Suricata ends a UDP flow after 30 s of quiet; counting
  every datagram as a contact (with the 1 s merge TCP uses) would make a QUIC download look
  like a perfect clock.
- **Chosen.** The flow-timeout idea with a shorter quiet spell: a datagram opens a contact
  when the device has sent nothing to that destination for 10 s, the shortest interval the
  scorer accepts. Zeek's minute would merge a 30 s check-in into one endless session. The
  contact then goes through the same RITA scoring as a TCP SYN, sizes included.
- **Tested.** A datagram every 50 ms for ten minutes is one contact and no rhythm; a burst
  of three datagrams every two minutes with jitter is one contact per burst, scored at about
  120 s and told once as `beacon-suspect`; a session still going counts as the device's
  latest reach, so the pruning of quiet pairs does not drop it.
- **Measured.** `Audit::see` in release, best of five runs of 500 000 packets: 262 ns for a
  TCP packet mid-stream, 315 ns for a UDP datagram mid-session (one more map lookup; the
  key is cloned only when a contact opens). No new state beyond one timestamp per
  device-destination pair already tracked.
- **Not adopted.** Zeek's 1 min timeout (hides check-ins faster than a minute), per-datagram
  contacts (streams look like clocks).

### 2026-10-16: destinations named by the DNS answers (passive DNS)

- **Compared.** RITA v5 groups beacons by FQDN, joining Zeek's `dns.log` answers to its
  connections; Sentry named an address only by the TLS SNI of a connection to it, which
  misses plain HTTP, encrypted ClientHellos (ECH) and the first contact to each new address
  of a CDN. A reverse lookup (PTR) of each address was the third option: active, slow, and
  a CDN's PTR names the CDN, not the service.
- **Chosen.** Passive DNS: TShark now prints `dns.a` and `dns.aaaa`; an answer from port 53
  names each outside address it gives with the name asked for (lowercased, without the
  trailing dot), at most 16 addresses an answer. SNI still names an address when it is
  seen. A malware controller that changes address every call then adds up to one
  destination, scored and told by its name. LAN addresses in answers are not named.
- **Tested.** Parsing (A and AAAA, invalid entries dropped, mDNS answers ignored); a device
  that resolves `c2.example.net` each minute and gets another address each time is told as
  `beacon-suspect` for `c2.example.net`, and no per-address destination is kept; the real
  TShark test reads the fixture with the two new fields.
- **Measured.** TShark over the 178-frame fixture, 20 runs each, alternated: 143/149 ms with
  the old fields, 148/141 ms with the new ones, within noise. In the audit, one map insert
  per address of a DNS answer; nothing for other frames.
- **Not adopted.** PTR lookups (active, and they name the CDN). Following CNAME chains:
  the name asked for is the one the device meant, which is what a beacon is grouped by.
- **Limit.** Sentry sees answers only to the machine it runs on, unless it sits on the
  gateway or behind a mirror port.

### 2026-10-16: domain generation algorithms (NXDOMAIN bursts)

- **Compared.** Pleiades (Antonakakis et al., USENIX Security 2012) clusters the NXDOMAIN
  answers a host gets by the shape of the names: accurate, but a trained model. The usual
  SIEM rule (Splunk's `ut_shannon` above 3 bits on queried names) flags single names and is
  noisy: `crashlyticsreports` scores 3.42. A plain NXDOMAIN count (Zeek scripts, Security
  Onion) fires on a Pi-hole set to answer NXDOMAIN for blocked trackers.
- **Chosen.** Count, per device, the different names it is told do not exist within a
  minute, keeping only names whose first label looks generated: 10+ characters, entropy of
  3 bits or more and under 30 % vowels. Tried on 26 common tracker and service labels, 3
  pass (`connectivitycheck`, `crashlyticsreports`, `ingest-sentry`); 72 to 75 % of random
  10 to 18 character labels pass. A blocked tracker repeats the same few names, so it
  cannot reach 25 different ones; a DGA tries new ones each time. The device is the
  answer's Ethernet destination, so no ARP needs to have been heard. A warning, told at
  most once an hour per device.
- **Tested.** `random_label` on generated and chosen labels; NXDOMAIN parsing (rcode 3 from
  port 53, by Ethernet destination); a Pi-hole and Chrome's three intranet probes repeated
  300 times stay quiet; a bot trying a name every half second is told once. The real
  TShark test reads a fixture where the router answers NXDOMAIN to the Windows client for
  40 generated names (29 pass the filter) and finds `dga-suspect` for it.
- **Measured.** TShark over the fixture repeated 50 times (10,900 frames), best of five,
  two rounds: 429/409 ms with the 39 fields before DNS answers were read, 400/387 ms with
  41, 395/393 ms with all 42: within noise. In the audit, one label check per NXDOMAIN
  answer and nothing else.
- **Not adopted.** A model (Pleiades, a character n-gram classifier): more to ship and
  explain for a home network. Entropy alone: it flags real tracker names.
- **Limit.** Sentry sees only the answers to the machine it runs on, unless it sits on the
  gateway or behind a mirror port. A DGA built from dictionary words passes as chosen
  names.

### 2026-10-16: tunnels grouped by the registered domain (Public Suffix List)

- **Problem.** `dns-tunnel-suspect` is told once per zone, and the zone was the last two
  labels: every tunnel under `co.uk`, `com.br` or `github.io` was one zone, so a second
  tunnel there went untold.
- **Compared.** In a scratch crate, release build, 1,000,000 lookups: `psl` 2.1.241
  (MIT/Apache, the list compiled to match code, one types crate) builds in 18 s, answers in
  34 ns and adds about 0.9 MB; `psl2` 0.1.32 builds in 9 s but pulls the ICU crates by
  default, answers in 97 ns and needs lowercase input; a hand list of second-level
  suffixes (`co`, `com`, `org`…) misses private suffixes such as `github.io`. Both crates
  gave `example.co.uk`, `evil.com.br` (`psl2` only once lowercased) and `foo.github.io`.
- **Chosen.** `psl`: no ICU, fastest, correct on the cases above. The registered domain
  falls back to the whole name when the list has none.
- **Tested.** `registered` on ICANN and private suffixes and a single label; two tunnels
  under `co.uk` are now two alerts.
- **Measured.** The lookup runs only for a question already judged to carry data, so
  nothing on the common path. The list is as current as the crate version; it is updated
  with the crate.

### 2026-10-17: the router's connection tracking (roadmap stage 2, third step)

- **Compared.** Linux's `/proc/net/nf_conntrack` (read with the same pinned key, no new
  service); `conntrack -L` from conntrack-tools (same data, not installed on most stock
  firmware); NetFlow/IPFIX exported by the router (`softflowd`, needs a package and a
  collector on Sentry's side, and pushes a stream where a pull every five minutes does);
  SNMP `ipNetToMediaTable` (neighbours only, no connections). pf's state table on pfSense
  and OPNsense needs root.
- **Chosen.** `/proc/net/nf_conntrack`, the first 2,048 lines, appended to the read-only
  command after a separator. Per LAN address: connections to outside addresses, bytes sent
  in the original direction when counted, connections per peer. The audit keeps the latest
  view on the device (`router_flows`, eight busiest peers, named where Sentry knows a name)
  and never adds it to `sent_to`, so a device Sentry also hears is not counted twice.
- **Tested.** Parsing (TCP and UDP, with and without byte counts, LAN to LAN, the router's
  own WAN connection, IPv6 and junk lines skipped); the full read with all three sections;
  the pinned command on every kind, still with nothing that writes; the busiest peers
  first and by name, no device made from connections alone, the field surviving a save and
  a load, and a file with too many peers refused.
- **Measured.** 2,048 lines (460 KB) parsed in under 1 ms in release, best of 20, once
  every five minutes on the router thread: nothing on the capture path.
- **Not tested here.** No real router: the line format follows the kernel's, as OpenWrt
  prints it. Whether byte counting is on must be checked on the owner's router.

### 2026-10-17: passwords sent in the clear, by the protocol's own command

- **Compared.** `cleartext-login` flags a device opening port 21 or 23: blind to POP3, IMAP
  and SMTP, to a server on another port, and it cannot tell a login from a connection
  that never sends one. Zeek's `ftp`/`pop3` analyzers and credential sniffers (dsniff,
  Ettercap, PCredz) read the login command and keep the credential. Reading
  `http.authorization` would bring HTTP Basic, but that field carries the credential.
- **Chosen.** The dissectors' command fields only: `ftp.request.command`,
  `pop.request.command`, `imap.request.command`, `smtp.req.command`. A client sending
  `PASS`, `APOP`/`AUTH`, `LOGIN`/`AUTHENTICATE` or `AUTH` in the clear is
  `cleartext-password`: high when the server is outside the LAN, a warning for one inside
  (a NAS). Told once per device, server and protocol. The argument is never asked for, so
  no password reaches Sentry. HTTP Basic is left out for that reason. SMTP `AUTH` after
  STARTTLS is encrypted and not seen, so what is seen really is in the clear.
- **Tested.** The four protocols and their non-login commands (`USER`, `EHLO`); severity
  by where the server is, told once. Through real TShark: the fixture now has the Windows
  client sending POP3 and FTP passwords to an outside server, told as two high alerts.
- **Measured.** TShark over the fixture repeated 50 times (11,050 frames), best of five,
  two rounds: 403/406 ms with 42 fields, 403/416 ms with 46. Within noise.
- **Not adopted.** Keeping or showing any part of the credential, as the sniffers do; HTTP
  Basic (needs the credential field).

### 2026-10-17: password guessing (login bursts)

- **Compared.** Zeek's `ssh/detect-bruteforcing` counts failed SSH logins (30 within 30
  minutes) by inferring the outcome of each session; Suricata's ET "Potential SSH Scan"
  counts SYNs to port 22 (5 in 120 s), which a person reconnecting trips; fail2ban works on
  the server's own logs, which Sentry does not have. `port-scan` and `host-sweep` already
  cover many ports of one host and one port of many hosts, not many tries at one door.
- **Chosen.** Count connections a device opens to each host's login port (21, 22, 23,
  2323, 3389, 5900) in the same one-minute window the scan rules use; the 30th is
  `login-burst`, high, once per device, host, port and window. Zeek's 30 kept as the
  number, Suricata's SYN counting as the signal, with the minute so a person's handful of
  sessions never reaches it. It works inside the LAN (a camera attacking the NAS, as Mirai
  spreads) and outward (an owner's device attacking someone else's).
- **Tested.** An admin's ten SSH sessions and a browser's hundred HTTPS connections stay
  quiet; forty logins spread at one every three seconds never fill a window; a camera
  opening Telnet ten times a second is told once (with `cleartext-login` for Telnet
  itself). Through real TShark, the fixture's camera opening 35 Telnet connections to a
  NAS in under four seconds is told.
- **Measured.** One map update per connection opened to one of the six ports, inside the
  existing per-device window; at most 64 host and port pairs per device and window.
  Nothing for other traffic.
- **Limit.** An encrypted protocol hides whether the logins failed, so a tool that opens
  many sessions on purpose (Ansible without `ControlPersist`) looks the same.

## How a tool is accepted

A tool joins Sentry when, on a recorded pcap of the owner's network and on the fixtures in
`core/crates/archie-lan/tests/fixtures/`, it finds something the current path misses or
finds the same thing cheaper, and its cost on the gaming budget in
[SENTRY-ROADMAP.md](SENTRY-ROADMAP.md) is measured, not assumed.
