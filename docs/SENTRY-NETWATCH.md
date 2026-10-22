# Sentry: the continuous audit of the network

`archie-netwatch` listens to the traffic on the owner's network and keeps a record of
what each device does on it. From that record it raises alerts.

```bash
archie-netwatch interfaces
archie-netwatch live --interface eth0 --state-dir /var/lib/archie/netwatch
archie-netwatch live --interface eth0 --state-dir ./audit --learn 0 --seconds 30
archie-netwatch read capture.pcapng --state-dir ./audit
archie-netwatch report --state-dir /var/lib/archie/netwatch --alerts 50
archie-netwatch leases dhcp.leases --state-dir /var/lib/archie/netwatch
```

`live` runs until it is stopped, or until `--seconds 1..86400` ends the capture.
The optional limit uses TShark's autostop and saves state at EOF even if no packets
arrive. It measures capture time, not subprocess startup: Core must retain its
process timeout, especially for a remote extcap connection. Invalid/missing limits
and unknown live options are rejected before creating state or starting TShark.
`read` audits a capture file by the same rules, on the
frames' own clock. `report` prints the devices and the most recent alerts as JSON.

## What it reads

TShark does the capture and the dissection (`wireshark-cli`). From each frame it is asked
for the time, the length, hardware and IP addresses, the sender an ARP frame names, the address an IPv6 Neighbor Advertisement answers for, TCP
and UDP ports, the SYN and ACK flags, and three names: a DNS question, a DHCP host name
and a TLS server name. No payload is asked for, and only the first 768 bytes of a frame
are copied off the interface.

## What it keeps

`state.json`, per device (by hardware address): first and last seen, addresses, host
name, frames and bytes sent, the names it asked DNS for (up to 512) and the bytes it sent
to each place outside the network (up to 64, by server name where one was seen). Also
which device last claimed each IPv4 address.

`alerts.jsonl`, one alert to a line; past 4 MiB it becomes `alerts.1.jsonl`.

`told.json`: which alerts were already announced, and for `address-moved` how many
changes were counted. It is what makes a restart silent about what was already said.
Both `told.json` and `state.json` load through bounded parsers: a file beyond the
caps (4096 devices, 512 names, 64 peers, 16 addresses per device, 256 MiB) is refused
with an error that names the cap, never loaded partly.

On Unix the files are created with mode 0640 (subject to umask), so their owner's
group can also read them; the service uses group `archie`. Which names a device asks
for says a great deal about the people who use it. Windows deployments need account
ACLs; Unix mode bits are not a Windows privacy guarantee.

## Alerts

| Rule | Severity | Raised when |
|---|---|---|
| `new-device` | notice | a hardware address not seen before sends from a LAN address, after the learning time (`--learn`, 600 s) |
| `address-moved` | high for the router's address (v4 or v6), else notice | an address is claimed by a different device than before: IPv4 over ARP, IPv6 by a Neighbor Advertisement (`icmpv6.nd.na.target_address`) |
| `port-scan` | high | one device opens connections to 100 ports of one host within a minute |
| `host-sweep` | high | one device opens the same port on 50 hosts of the network within a minute |
| `cleartext-login` | warning | a device opens Telnet or FTP |
| `dns-tunnel-suspect` | warning | a DNS question is over 180 characters, or has a label of 40 or more near-random characters |
| `unusual-destination` | warning | a device learned for a day sends over 25 MB to an outside destination (server name, else address) first used after it was learned; told once per device and destination. Which destinations are new is kept in memory, so a restart can only make it quieter; past the 64 destinations a device keeps, a new one is not tracked and cannot be told |
| `traffic-surge` | warning | a device learned for a day (144 ten-minute buckets) sends, in one bucket, more than all of: 50 MB, 4× its usual, and its usual plus 6 deviations; told at most once an hour per device. The usual is an exponentially weighted mean and variance kept in `state.json` (`baseline`), so a restart keeps it. Thresholds are a criterion, not a measurement: a nightly backup will trip it until the false-positive rate is measured on a real capture |
| `rogue-dhcp-server` | high | a second device answers DHCP (an offer or acknowledgement) once one is known, after the learning time: it can hand clients its own router and DNS. Every server seen is kept in `state.json` (`dhcp_servers`) |
| `lease-conflict` | warning, once per address and device | a device claims over ARP an IPv4 address that the router's DHCP server leased to another device, while that lease holds; checked both when the claim is heard and when a lease table is read after it. Either a static address set inside the DHCP range (two devices then fight for it) or a device taking another's traffic. Needs a lease table (`leases`); the table read last is kept in `state.json` (`leased_to`) |
| `rogue-dns-server` | warning, once per device | after the learning time, a device on the LAN that is not the router (ARP owner of `--gateway`) answers DNS (from port 53) to another device on the LAN. Servers seen while learning, such as a Pi-hole or a NAS, are the network's own and kept in `state.json` (`dns_servers`); answers to the internet and mDNS (5353) are not counted. Answers are unicast: on a switched network Sentry sees those to its own machine |
| `arp-sweep` | warning, at most once an hour per device | one device asks ARP (`who has …`, not its own gratuitous announcement) for 50 or more different addresses within a minute: a scan of the network such as `nmap -sn`, `arp-scan` or `netdiscover`, or a network app on one of the owner's phones. ARP requests are broadcast, so unlike most rules this one sees every device even on a switched network. Up to 256 devices are counted at once |
| `dga-suspect` | warning, at most once an hour per device | the device's DNS questions come back NXDOMAIN for 25 or more different names within a minute whose first label looks generated (10+ characters, 3+ bits of entropy, under 30 % vowels): malware with a domain generation algorithm trying names until one reaches its controller. Blocked trackers repeat a few names and do not reach it. The device is the answer's Ethernet destination; only answers Sentry hears count, so on a switched network that is the machine it runs on |
| `cleartext-password` | high when the server is outside the LAN, warning inside it; once per device, server and protocol | the device sent a login command unencrypted, on any port: FTP `PASS`, POP3 `PASS`/`APOP`/`AUTH`, IMAP `LOGIN`/`AUTHENTICATE`, SMTP `AUTH`. Sentry reads only the command word from TShark, never the password itself. HTTP Basic is not checked, since its field carries the credential |
| `login-burst` | high, once per device, host, port and minute | the device opens 30 or more connections to one host's login port within a minute: FTP (21), SSH (22), Telnet (23, and 2323 as Mirai uses), RDP (3389) or VNC (5900). Password guessing, by a tool or by malware spreading through the network. Encrypted logins hide whether they failed, so a tool that opens many sessions on purpose looks the same |
| `retired-smb` | warning, once per device | the device sends SMB version 1 commands other than the opening Negotiate, so it and its peer agreed on SMBv1: the protocol EternalBlue and WannaCry spread through, off by default in Windows since 2017. Usually an old NAS, printer or camera. A Negotiate alone is not counted, since modern clients open with one and move to SMB2 |
| `dns-bypass` | notice, once per device and resolver | the device resolves names outside the network's own DNS: a DNS question to port 53 of an outside address, a connection to DNS over TLS (853), or DNS over HTTPS to a public provider (Google, Cloudflare, Quad9, OpenDNS, CleanBrowsing, AdGuard, Mullvad, NextDNS) by its TLS server name. A Pi-hole or the router's own filtering does not see those lookups. The router and LAN DNS servers forward upstream and are not told |
| `icmp-redirect` | high when the sender is not the router (the ARP owner of `--gateway`, or of the IPv6 gateway), warning while the router is not known yet; once per sender | a device sends an ICMP Redirect (IPv4 type 5, IPv6 type 137) telling a host to send traffic for some destination through another router. The router may do that; anyone else is putting itself in the middle without touching ARP, so `address-moved` does not see it. Redirects are unicast: on a switched network Sentry sees those sent to its own machine |
| `retired-tls` | notice, once per device | a device's TLS ClientHello offers nothing newer than TLS 1.1 (read from its JA4: `t10…`, `t11…`, `ts3…`). RFC 8996 retired TLS 1.0 and 1.1 in 2021, so such a client is firmware that has not been updated in years, most often a camera, a DVR or an appliance. The alert names the server it was talking to; each device's `profiles` entry lists the TLS versions its clients offered (`tls_versions`) |
| `rogue-dhcpv6-server` | high when it is not the device that holds the IPv4 router's address over ARP (`--gateway`), else warning | a device answers DHCPv6 (an advertise or reply, from UDP 547) after the learning time, and it is not the router, or other servers were known before it. Windows prefers IPv6 and asks DHCPv6 even on networks that use none; mitm6 answers and becomes every Windows machine's DNS server. Every server seen is kept in `state.json` (`dhcpv6_servers`) |
| `name-poisoning` | high, once per device | a device answers LLMNR (UDP 5355) or NetBIOS name service (UDP 137) queries for three different names, or for `wpad`. Windows asks the whole link for names DNS does not know; Responder answers every one of them to take the connection and collect password hashes, and answering `wpad` makes it the browsers' proxy. An honest machine answers for its own name. Answers are unicast to the asker: on a switched network Sentry sees those to its own machine, and all of them on a gateway or mirror port. mDNS answers are not counted: a device announces many service names there |
| `mac-flood` | high, at most once an hour | 20 or more hardware addresses that are not known devices send frames within a minute, once the sensor has run a minute (or its learning time, if longer). A tool like `macof` fills a switch's address table so the switch repeats everyone's traffic to every port. While it lasts, unknown addresses are not kept as devices, so made-up ones cannot fill the inventory and their `new-device` alerts are not raised; known devices are still followed |
| `dhcp-starvation` | high, at most once an hour | 30 or more hardware addresses that are not known devices send a DHCP discover or request within a minute (the client address is the DHCP `chaddr`, `dhcp.hw.mac_addr`). Tools like Yersinia and DHCPig drain the router's pool this way so a rogue server answers next; the alert names the Ethernet sender that carried the most of them and says when one sender is behind them all. A home network's busiest minute is far below 30 new devices; renewals by known devices do not count |
| `rogue-router-advertisement` | high when it is not the router (not `--gateway`'s IPv6 address, or not the device that holds the IPv4 router's address over ARP), else warning | a device offers itself as an IPv6 router (ICMPv6 type 134). On a network whose router offers no IPv6, the first advertisement is how a device makes itself everyone's router |
| `beacon-suspect` | warning | a device learned for a day opens connections to an outside destination it took up after it was learned at a clock-like interval: at least 12 contacts (connections within 1 s are one), median interval 10 s to 24 h, scored 85 or more of 100 as RITA does: the intervals between contacts and the bytes sent between them, each with Bowley skewness near zero and a median absolute deviation small against the median, averaged (intervals alone until 11 sizes are known). Rhythms scoring 70 or more to any destination are kept on the device in `state.json` (`beacons`: interval, score, median bytes, first and last time) without an alert, since NTP and vendor keep-alives are clocks too. Only TCP connections are timed; the last 33 contact times of up to 4096 device and destination pairs are kept in memory |

An alert is a reason to look, with what was seen. It is not a verdict, and nothing here
acts on a device or on the network. Each is told once for its subject, and the telling
survives a restart.

## What devices say about themselves

Beside the rules, each device keeps what its own frames say in the clear, and `report`
lists it under `profiles`, read by `archie_lan::fingerprint`:

- **Maker**, from the IEEE OUI registry (`oui-data`, embedded at build time), unless the
  hardware address is locally administered (random, as phones and laptops use per network),
  which is reported as such.
- **DHCP**: the options its client asks for, in order (equal strings mean the same DHCP
  stack), and its vendor class (`MSFT 5.0`, `android-dhcp-14`, `dhcpcd-…`, `udhcp…`). A
  client's discover and request come from 0.0.0.0, so they are kept for its hardware
  address until the device is first seen with an address; LLDP announcements likewise.
- **TCP**: the TTL its connection openings most likely started from (64, 128, 255) and the
  window it advertised.
- **TLS JA4** of its ClientHellos (computed by TShark), **HTTP User-Agents**, its **LLDP**
  system name and **NetBIOS** names.

From these, `hints` say what the device may be, each with the evidence it rests on ("Windows"
because of the vendor class `MSFT 5.0`; "Windows" because its connections start with a TTL of
128). They are hints, never verdicts: a vendor class can be set by hand and a random address
hides the maker on purpose. Nothing is sent to any device to learn them.

`tests/fixtures/rogue-dhcp-and-ra.pcap` (written by `make_rogue_pcap.py` with scapy) holds a
Windows client, the router, an LLDP switch, and a second device that answers DHCP and
advertises itself as an IPv6 router; `tests/netwatch_tshark.rs` reads it through a real
TShark (`--ignored`, it needs TShark on the PATH). With TShark 4.2.2 both rogues are told as
high, and the Windows client's maker, DHCP fingerprint, vendor class, TTL, JA4, NetBIOS name
and User-Agent are all kept.

## What the router knows

On a switched network Sentry hears a device only when it broadcasts. The router hands every
device its address, so its DHCP lease table names them all. `leases` reads that table in
any of three formats, told apart by their shape:

- dnsmasq's file (`<expiry> <mac> <ip> <hostname|*> <client-id|*>`): OpenWrt
  (`/tmp/dhcp.leases`), Pi-hole, DD-WRT, Tomato, Asuswrt-Merlin;
- ISC dhcpd's `dhcpd.leases` (`lease <ip> { … }` blocks; only `binding state active`, the
  last block for an address holds): pfSense (`/var/dhcpd/var/db/dhcpd.leases`), OPNsense
  with the ISC backend, Linux servers;
- OpenWrt's `ubus call luci-rpc getDHCPLeases` output (JSON; its `expires` counts seconds
  from when it was taken, so the file should be read soon after).

```bash
scp root@192.168.1.1:/tmp/dhcp.leases . && archie-netwatch leases dhcp.leases --state-dir DIR
```

Each lease adds the device (if new), its address and the router's name for it, and records
when the lease ends (`leased_until_s`; 0 for a lease that does not end). A lease is not a
sighting: `last_seen_ms` is left as the wire last saw it. Once the learning time is over
(`--learn`, 600 s from the first frame or lease the state holds), a device that only the
router knows is told as `new-device`, once, whether its first appearance is a lease or a
frame. IPv6 leases and lines of another shape are skipped; files over 1 MB are refused.

### Enrolling the router (read over SSH)

Copying the file by hand is the fallback. `router` enrols the owner's router once and from
then on `live` reads it every 5 minutes, on its own thread so a slow SSH never holds up the
capture:

```bash
archie-netwatch router detect                      # what the gateway runs (SSH banner, web title)
archie-netwatch router enroll --state-dir DIR --install   # owner types the router password once
archie-netwatch router pull --state-dir DIR        # one read now
```

- `detect` reads the gateway's SSH banner and web page title and names the kind: OpenWrt,
  Asuswrt, a BSD firewall (pfSense, OPNsense) or plain Linux.
- `enroll` makes an ed25519 key in the state directory (`router_ed25519`, private to the owner as `ssh-keygen` writes it), writes
  `router.json` and prints the `authorized_keys` line. The key is pinned to one read command
  (`command="…",no-port-forwarding,no-agent-forwarding,no-X11-forwarding,no-pty`): the
  lease table, then the neighbour (ARP) table. Whatever else is asked of the key, the router
  runs only that.
- `--install` adds the line over `ssh` itself; `ssh` asks the owner for the router's
  password once, on the terminal, and Sentry never sees it. Asuswrt and pfSense/OPNsense
  keep keys in their own settings page; for them `enroll` says where to paste the line.
- The router's host key is pinned on first contact (`router_known_hosts`); a later change
  fails the read instead of trusting the new key.
- Reads use the enrolled key only (`BatchMode`, `IdentitiesOnly`, no agent), time out after
  20 s, and feed the audit like a lease file, plus the router's neighbour table: every device
  the router has talked to, including those a switch hides from this machine. Neighbours go
  through the same address checks as ARP heard on the wire (`address-moved`,
  `lease-conflict`).

- On OpenWrt, Asuswrt and Linux routers the read also takes the first 2,048 lines of the
  kernel's connection tracking (`/proc/net/nf_conntrack`). Per device (matched by address
  through the same read's lease and neighbour tables), `router_flows` keeps the number of
  connections to the outside, the bytes sent over them when the kernel counts bytes
  (`nf_conntrack_acct`), and the eight busiest destinations, by name when Sentry knows one.
  It is the latest view, replaced at each read, and adds nothing to `sent_to`, which counts
  only what Sentry itself heard. pfSense and OPNsense keep connections in pf, whose state
  table needs root, so they are not read.

Nothing is changed on the router: the key can only read. A key enrolled before the
connection tracking was added keeps its old command until it is enrolled again.

## Running it as a service

`archie-netwatch.service` is installed and not enabled. Write the interface into
`/etc/archie/netwatch.env` and enable it:

```bash
echo 'ARCHIE_NETWATCH_INTERFACE=eth0' | sudo tee /etc/archie/netwatch.env
sudo systemctl enable --now archie-netwatch
```

It runs as the static account `archie-netwatch` (`distro/sentry/archie-netwatch.sysusers`)
with `Group=archie`, holding only `CAP_NET_RAW` and `CAP_NET_ADMIN`, which is what
`dumpcap` needs to open the interface. Why not `DynamicUser`:

- Arch's `wireshark-cli` installs `dumpcap` as `root:wireshark` mode 0754, so the
  account must be in `wireshark` (`SupplementaryGroups=wireshark`) even to execute it.
- A dynamic user's `StateDirectory` lives under `/var/lib/private` (0700 root), where
  `archie sentry-follow`, running as `archie`, could never read `alerts.jsonl`.

Its files are 0640 in a 0750 directory: the control plane (group `archie`) reads them,
nobody else does. `systemd-analyze security` puts the unit at 3.5 (it was 5.1 before
this change). Not verified on a running Arch system: that `dumpcap` opens the interface
under these settings with `NoNewPrivileges=yes`.

## What it can see

A machine on a switch sees its own traffic, broadcasts and multicast: ARP, DHCP, mDNS,
and whatever is addressed to it. That can reveal `new-device` and `address-moved`
for nodes whose announcements actually reach it, not every node in the whole network.
Sleeping devices, client isolation and VLANs can leave gaps. WSL's virtual interface
may expose only its virtual neighbors and transit traffic. To audit other
device's traffic the node has to be where that traffic passes: the router or gateway
itself, a mirror port on the switch, or a tap.

## Limits

- Not verified as a systemd unit or inside the ISO. Verified live on Linux, as root, on
  one interface (see the hand-off for the run).
- Encrypted DNS (DoH, DoT) hides the questions; only the server name of the resolver shows.
- `address-moved` follows an address, not a pair. Two devices fighting over one address
  raise one alert whose count grows; a further alert comes only after an hour of cool-down
  and at least one more change.
- A device that changes its hardware address (phones do, per network) is a new device.
- The inventory, this record and the passive import are joined by `archie sentry-devices`
  into one view keyed by hardware address, and that view reaches Sentry's world graph.
  What is not joined yet: the QML panel of the Observatory does not draw them yet (the
  graph and the display data path carry them; the visual panel is pending).
