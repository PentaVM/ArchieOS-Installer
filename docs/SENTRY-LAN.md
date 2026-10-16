# Sentry on the local network

`archie-lan` discovers devices through protocol observations. It preserves
`archie-lan-inventory-v1`, the Core unified device view and the existing Sentry feeder.
Discovery never grants permissions, pairs devices or replaces Core's authority.

```sh
archie-lan scan --seconds 12 --diagnostics ./coverage.json
archie-lan scan --seconds 12 --probe-local --probe-seconds 75 --diagnostics ./coverage.json
archie-lan interfaces
archie-lan scan --bind LOCAL_IPV4 --subnet PRIVATE_CIDR --probe-seconds 75
archie-lan watch --out /var/lib/archie/sentry/lan.json --every 300 --seconds 12
archie-lan watch --autonomous --out ./lan.json --probe-local --diagnostics ./lan.coverage.json
archie-lan watch --autonomous --once --out ./lan.json --seconds 12
archie-lan scan --mdns-backend hybrid --seconds 12
```

`--bind` selects an assigned private IPv4 interface. With no binding, Sentry uses
its default-route source address. `--probe-local` obtains that interface's subnet
mask through Sentry's own interface API; on a network wider than /24 it sweeps only
the /24 around the selected address.
`--subnet` is an explicit private /24..=/32 scope containing the chosen interface.
Native discovery uses IPv4. The optional `mdns-sd`/`hybrid` backend adds private
IPv6 DNS-SD observations on the selected interface; HTTP and TCP remain IPv4.
Other VLANs and arbitrary routed networks are not covered.

## Sources and evidence

| Source | Observation |
| --- | --- |
| SSDP / UPnP | Published device description, manufacturer and model |
| mDNS unicast and shared multicast | Services, instance names, SRV ports and selected TXT facts |
| Optional mdns-sd / hybrid backend | Upstream DNS-SD resolution, scoped private IPv4/IPv6 service claims; no derived actuator URI |
| Explicit DNS-SD queries | Printer/IPPS, media, Matter, HomeKit, MQTT, ESPHome, WLED, Shelly, Hue, host and Android debug services |
| SRV/TXT/A/AAAA follow-up queries | Resolve service-only advertisements without assuming that PTR includes all records |
| WS-Discovery 2005/04 and 1.1 | Correlated WSD printer / ONVIF camera claims; unknown types remain unknown |
| Optional TCP presence | Open-port observations, without application payloads or device classification |
| Sentry neighbor-cache collector | Interface-scoped private ARP candidates, MAC and cached status |
| Roku ECP | Power and current application reported by the receiver |
| Spotify ZeroConf getInfo | Declared device type and model from an advertised local endpoint; no account identifiers |

A printer URI requires a matching SRV/TXT instance and A/AAAA record for the
announcing address. Public or foreign WSD metadata addresses are not emitted.
WSD endpoints are metadata endpoints, not IPP printing URIs. Advertisements are
not authenticated; a later paired adapter still validates its identity.

`discovery_tags` describes advertised protocols. A Matter advertisement does not
prove a bulb, and a hub does not enumerate every child device without its controller
integration. Spotify or companion-link alone does not identify a person's phone.
`local_address_matches=true` states that an announcement address matches Sentry's
selected local interface; it is not an ownership assertion.

## Cache candidates and freshness

Sentry itself reads a bounded ARP cache, not the Windows printer/device inventory.
Neighbor-only records have no proven live reachability, name, vendor, model or owner.
Core preserves unknown observation timestamps for those records; the live context
feeder must not generate fresh device-seen events from a cache-only candidate.
When another discovery source responds, that source supplies current observation.

## Autonomous monitor

`watch --autonomous` combines native discovery with the pinned upstream `mdns-sd`
backend on deep passes; light passes use native discovery only, and the deep sources'
observations stand until the next deep pass. Light passes run 30 seconds after the
previous pass finishes;
TCP passes, when explicitly enabled by `--probe-local`/`--subnet`, are scheduled
separately every 600 seconds with a default 75-second budget. No passes overlap.
`modules/sentry/lan.toml` enables this worker under Core's existing permissions.
Large local subnets are swept in the /24 around this node. The legacy scan/watch defaults stay native and retain their
15-second TCP budget; legacy watch cadence stays 300 seconds.

Default sidecars are `lan.history.json` and `lan.health.json`; `--history` and
`--health` override them. The history is limited to 4096 entries and 8 MiB, retains
observations for one day, and survives process restarts. Historical rows retain
descriptive metadata, clear old power/app/session state, and use `history-cache`
provenance. Core and the live feeder never turn them into fresh presence. Address,
netmask or interface changes reset the old scope; identical IP configurations on
different physical networks are not a verified device identity.

Failed scans preserve the successful inventory, persist health and retry with
exponential delay (default 30–300 seconds). Publication failures also retry; if
health/state cannot be written, stderr reports it. Corrupt startup history fails
explicitly rather than silently replacing the owner's files. File locks prevent
another cooperative monitor from owning the same outputs/state; colliding paths
are rejected. Each file is replaced atomically, but the sidecars are not a single
multi-file transaction. Unix files use mode 0600; Windows inherits the directory
ACL. Use a private owner directory. `--once` performs one iteration and exits.

Upstream research, integration choices and physical limitations are documented in
[SENTRY-AUTONOMY.md](SENTRY-AUTONOMY.md).

## Coverage and limits

The diagnostics sidecar reports availability, errors and result counts per source,
plus subnet, attempted hosts and port probes, and whether TCP coverage completed.
An unavailable source does not discard successful sources. An invalid local binding
must fail without replacing a watch inventory. Multicast unavailability preserves
unicast discovery and is explicitly reported.

TCP probing is opt-in: at most 256 addresses, 16 workers, 250 ms per connection.
The default overall deadline is 15 seconds, configurable from 1 to 120 seconds.
Scheduling visits every host at the first port before moving to the next port.
A deadline can leave partial port coverage even when every host was attempted.
No bytes are sent to raw printing port 9100 or other presence ports.

Captured/decrypted traffic from all devices still requires a suitable router,
mirror port or tap. WSL virtual networking may expose only its virtual segment.
A sleeping, isolated, cloud-only or non-announcing device with no sampled open port
can remain unknown. Zigbee, Bluetooth and Thread devices need their corresponding
controller or an already configured hub integration; these queries do not silently
commission them. No universal detection guarantee is claimed.

The v1 inventory is local private data; do not upload home addresses/MACs to PRs.
Printing uses the existing Sentry printer flow and requires a discovered/enrolled
printer, supported document format, identity validation and the owner's grant.

## Protocol references

- [RFC 6762 — multicast DNS](https://www.rfc-editor.org/rfc/rfc6762)
- [RFC 6763 — DNS-SD](https://www.rfc-editor.org/rfc/rfc6763)
- [OASIS WS-Discovery 1.1](https://docs.oasis-open.org/ws-dd/discovery/1.1/os/wsdd-discovery-1.1-spec-os.html)
- [Spotify ZeroConf](https://developer.spotify.com/documentation/commercial-hardware/implementation/guides/zeroconf)

See `_dev/SENTRY-DISCOVERY-2026-10-12.md` for test and live-run evidence.
