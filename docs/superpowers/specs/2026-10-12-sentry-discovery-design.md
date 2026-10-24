# Sentry: broader LAN discovery

The owner wants Sentry itself to discover the home network without consulting a
preselected device list or encoding the owner's printer, phone, addresses or
vendors. Existing Core, permissions, journal, IRIS and Cortex remain authoritative.
The live baseline found three announcing endpoints; it missed the printer and phone.

## Design

Extend `archie-lan`, preserving the v1 inventory contract and existing feeder.
Keep SSDP and Roku observation. Add multicast mDNS reception alongside unicast,
direct DNS-SD queries for printer, media, Matter, MQTT and home automation services,
and WS-Discovery probes for WSD/ONVIF devices. Collect printer service endpoints
only from matching SRV/TXT evidence, and never treat a service as proof of ownership.

Add an opt-in bounded TCP presence scan of the selected interface's real local
subnet, or an explicitly supplied private CIDR. Maximum 256 addresses, 16 workers,
250 ms per connection and a 15-second default deadline, configurable up to 120
seconds. Port-first scheduling gives all addresses equal coverage before deeper
checks. TCP presence is evidence of
an open port, not proof of a vendor, model, bulb, phone or printer. Send no application
payload to presence-scan ports (especially raw printing port 9100).

Expose explicit interface binding and a diagnostics sidecar. Failures of one
discovery source must not discard successful sources. Do not change the inventory
schema: the sidecar carries coverage/errors and source counters; additive device
facts fit bounded existing `state` fields. Private results remain local.

Sentry itself also observes the selected interface's bounded ARP cache, retaining
cache status and unknown identity. Cache-only records must not acquire live peer
observation timestamps in Core or produce fresh device-seen events in the feeder.
This is an implemented source inside Sentry, not an agent's fallback device inventory.
Spotify's published ZeroConf getInfo may supply a declared device type from its
advertised same-host endpoint; no login, activeUser or deviceID is retained.

## Boundaries

No vendor account login, credential guessing, pairing, device control or automatic
printing. Discovery does not enroll devices or create grants. Existing reviewed
Sentry printing can be used for the already authorized greeting only if discovery
provides a printer endpoint and the adapter validates the target and formats.

Network observations are device claims, not authenticated identity. Spotify alone
does not identify a phone or its owner. Zigbee/Bluetooth/Thread devices may need an
existing hub/controller; isolated VLANs, sleeping devices and non-announcing devices
without probed open ports can remain invisible. WSL virtual networking may not expose
the physical LAN. IPv4 discovery is the deliverable; IPv6 mDNS/Thread transport is
explicitly deferred, not presented as supported.

## Proof

Test real DNS and SOAP parsing against hand-built packets, multicast socket reception,
service-only discovery without the meta query, printer endpoint association, spoofed
cross-host endpoints, public/oversized subnet rejection, bounded TCP detection and
unknown classification. Rebuild and run Sentry on the owner's PC, compare its actual
output with the saved baseline, and report what still cannot be detected.

References: RFC 6762/6763, OASIS WS-Discovery 1.1 and the 2005/04 discovery dialect.
User explicitly authorized direct implementation without further approval prompts.
