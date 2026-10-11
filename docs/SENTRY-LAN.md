# Sentry on the local network

`archie-lan` is Sentry's sense of the owner's own network: which devices are on it, what
kind of thing each is, and what the ones that offer it say they are doing.

```bash
archie-lan scan                                  # one inventory, as JSON
archie-lan watch --out /var/lib/archie/sentry/lan.json   # the service (module sentry.lan)
```

## How it finds things

| Source | What answers | What it gives |
|---|---|---|
| SSDP (`M-SEARCH`, UPnP) | televisions, routers, media players, game consoles | what the device says it is, and where it publishes its description |
| UPnP description | whatever published one | its name, maker and model |
| mDNS service discovery | printers, Chromecast and AirPlay devices, speakers, home hubs, computers | the services it offers, its name, and its model when it says |
| Roku's External Control Protocol | Roku players and televisions | power state and what is on the screen |

A device is given one of the families ArchieOS has adapters for (`television`, `router`,
`camera`, `printer`, `home-assistant`, `mobile`, `host`) when what it said supports one,
and is listed without a family otherwise.

On a machine with virtual network adapters the question is sent from the address this
machine's traffic leaves by, so it reaches the real network and not a hypervisor's.

## The inventory

`archie-lan-inventory-v1`: `scanned_unix` and one entry per device with `address`,
`class`, `name`, `manufacturer`, `model`, `services`, `seen_by` and `state`.

## What it does not do

- **Leave the LAN.** Only private and link-local addresses are spoken to or believed. A
  description is fetched only from the host that announced it.
- **Probe.** No port scan, no address sweep, no passwords, no vendor cloud. A device that
  does not announce itself is not found.
- **Act.** It reads. Changing a device is a separate, granted operation and is not here.
- **Name people.** A phone is a device of family `mobile`; whose it is, is not asked.
- **Keep identifiers it has no use for.** Serial numbers in descriptions and Roku answers
  are not kept.

## Limits

- One network: the one the default route uses.
- A device that answers mDNS only to the multicast group, and never directly, is not
  heard. SSDP finds most of those.
- Of the devices that report state, only Roku is read. Chromecast, Home Assistant and
  printers are found and classified, not yet asked.
- The inventory is a file; nothing puts it into Sentry's world graph yet.
