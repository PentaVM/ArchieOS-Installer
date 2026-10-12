# Sentry: the continuous audit of the network

`archie-netwatch` listens to the traffic on the owner's network and keeps a record of
what each device does on it. From that record it raises alerts.

```bash
archie-netwatch interfaces
archie-netwatch live --interface eth0 --state-dir /var/lib/archie/netwatch
archie-netwatch read capture.pcapng --state-dir ./audit
archie-netwatch report --state-dir /var/lib/archie/netwatch --alerts 50
```

`live` runs until it is stopped. `read` audits a capture file by the same rules, on the
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

All three are written for their owner only (mode 0600). Which names a device asks for
says a great deal about the people who use it.

## Alerts

| Rule | Severity | Raised when |
|---|---|---|
| `new-device` | notice | a hardware address not seen before sends from a LAN address, after the learning time (`--learn`, 600 s) |
| `address-moved` | high for the router's address (v4 or v6), else notice | an address is claimed by a different device than before: IPv4 over ARP, IPv6 by a Neighbor Advertisement (`icmpv6.nd.na.target_address`) |
| `port-scan` | high | one device opens connections to 100 ports of one host within a minute |
| `host-sweep` | high | one device opens the same port on 50 hosts of the network within a minute |
| `cleartext-login` | warning | a device opens Telnet or FTP |
| `dns-tunnel-suspect` | warning | a DNS question is over 180 characters, or has a label of 40 or more near-random characters |
| `traffic-surge` | warning | a device learned for a day (144 ten-minute buckets) sends, in one bucket, more than all of: 50 MB, 4× its usual, and its usual plus 6 deviations; told at most once an hour per device. The usual is an exponentially weighted mean and variance kept in `state.json` (`baseline`), so a restart keeps it. Thresholds are a criterion, not a measurement: a nightly backup will trip it until the false-positive rate is measured on a real capture |

An alert is a reason to look, with what was seen. It is not a verdict, and nothing here
acts on a device or on the network. Each is told once for its subject, and the telling
survives a restart.

## Running it as a service

`archie-netwatch.service` is installed and not enabled. Write the interface into
`/etc/archie/netwatch.env` and enable it:

```bash
echo 'ARCHIE_NETWATCH_INTERFACE=eth0' | sudo tee /etc/archie/netwatch.env
sudo systemctl enable --now archie-netwatch
```

It runs as a transient user holding only `CAP_NET_RAW` and `CAP_NET_ADMIN`, which is what
`dumpcap` needs to open the interface.

## What it can see

A machine on a switch sees its own traffic, broadcasts and multicast: ARP, DHCP, mDNS,
and whatever is addressed to it. That is enough for `new-device` and `address-moved` for
the whole network, and for the other rules only for this machine. To audit every
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
