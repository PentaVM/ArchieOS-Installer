# Sentry roadmap

Where Sentry is going: the owner's network task manager. One place that knows every device
the owner has (at home, at work, at a second house), what each one is doing, and can manage
the screens and cameras among them, without costing the owner's games a millisecond.

Sentry and Wanda are joined by a bridge: Sentry feeds Wanda what the network shows (who is
home, which device is awake), and Wanda never reaches back into the network through it.

## Rules that do not move

- **Owner's devices only.** Sentry inventories what it hears, but it only acts on, logs
  in to or configures a device the owner has enrolled as theirs. Neighbours, guests and
  anything on a network the owner does not run are observed only as far as their own
  broadcasts reach, and never probed.
- **Observation first.** Every change (a router setting, a camera stream, a firewall rule)
  is a proposal that goes through Policy and the owner's approval, with the exact change
  shown and a way back. Nothing is changed because a model thought it was a good idea.
- **No attacks.** No credential guessing, no exploits, no deauthentication, no ARP
  spoofing to get a view of traffic. A better view comes from the router or a mirror port
  the owner configures.

## Stages

1. **LAN inventory and audit** (done in this PR): passive discovery, fingerprints, alerts
   for new devices, rogue DHCP and rogue router advertisements, a JSON report.
2. **The gateway view.** Sentry reads what the owner's router already knows: its DHCP
   leases, ARP/neighbour table, connection tracking and per-device byte counts, through
   the router's documented API (OpenWrt `ubus`/LuCI RPC, UPnP IGD, SNMP, a vendor API).
   This sees every device, including those a switch hides from the operator's machine.
3. **Many sites, one inventory (WAN).** Each site the owner runs has its own Sentry
   sensor (the operator PC, a gateway, a small always-on box). Sensors join over an
   encrypted overlay the owner controls (WireGuard; Headscale if they want a coordinator)
   and report to one Core. Nothing is exposed to the internet: no port forwards, no UPnP
   openings made by Sentry.
4. **Screens and cameras.** Enrolled devices are managed through the protocols they
   publish: ONVIF and RTSP for cameras, DLNA/UPnP, Google Cast and the TV's own LAN API
   for screens. Sentry can show what is on, wake or turn off a screen, check a camera's
   firmware and stream health, and propose fixes (a default password, an open UPnP port,
   a camera talking to an unknown cloud). Configuring a camera is allowed when the owner
   approves it; live views go through the owner's own session, never stored by default.
5. **Router configuration with approval.** Proposed changes Sentry can draft and, once
   approved, apply through the router's API: isolate IoT on its own VLAN or SSID, block a
   device's outbound traffic, reserve a DHCP address, enable SQM (`cake`) against
   bufferbloat. Each change is recorded with the command that undoes it.
6. **Task manager.** Per device and per site: who is using the bandwidth, what woke up at
   3 a.m., which camera stopped streaming, which device has not been seen in a week. The
   orb and the operator panel are the views; Policy decides what Sentry may do about it.

## Gaming budget

Sentry must not be felt in a game. The budget, measured on the operator PC with a game
running:

| Measure | Budget | How |
|---|---|---|
| Added network latency | 0 ms | Sentry is passive: it copies frames, it is never in the forwarding path |
| Latency under load (bufferbloat) | must not rise | `flent rrul` / `irtt` with and without Sentry running |
| CPU | < 1 % of one core, average, while gaming | `pidstat` on `dumpcap`, `tshark`, `archie-netwatch` |
| Frame time | no change in 1 % lows | a fixed benchmark scene, Sentry on vs off |
| Disk writes | state saved at most once a minute | already the case |

When the operator PC is the gaming PC, heavy work (full captures, Zeek, Suricata) belongs
on the gateway or an always-on box, and the operator PC keeps only the broadcast listener.
Router SQM is the one change that usually makes games better; it is proposed, measured
before and after, and kept only if the numbers say so.

None of these numbers has been measured yet. They are not declared met until they are.
