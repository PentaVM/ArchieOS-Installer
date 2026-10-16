# Sentry autonomy and upstream components

The owner requested a more autonomous, capable observer and a GitHub library audit.
This extends the existing Nvidia LAN worker; it creates no independent Cortex or
authority. Keep native discovery and inventory v1, add an upstream DNS-SD backend,
then persist observation history and schedule cheap/deep passes independently.

## Upstream shortlist

| Project | Fit | Decision |
| --- | --- | --- |
| [mdns-sd](https://github.com/keepsimple1/mdns-sd) | Safe Rust DNS-SD, Windows/Linux/macOS, IPv4/IPv6; event channels and service resolution | Add a bounded supplemental backend. Keep native fallback. Pin the dependency and audit service conversion. MIT/Apache-2.0. |
| [btleplug](https://github.com/deviceplug/btleplug) | Native BLE central APIs across host platforms | Future optional observation source beside existing BlueZ adapter. BLE only; no Bluetooth Classic. Platform permissions/hardware still apply. |
| [rumqtt](https://github.com/bytebeamio/rumqtt) | Rust MQTT clients/event loop | Evaluate for configured broker telemetry; current MQTT adapter already exists. Never connect with guessed credentials or publish commands from discovery. |
| [Home Assistant](https://github.com/home-assistant/core) | Broad home-device integration ecosystem, Python implementation | Observe an existing configured hub through its API using a Rust client. Do not embed Python or delegate Archie's policy/planner to the hub. |
| [Zigbee2MQTT](https://github.com/Koenkk/zigbee2mqtt) | Zigbee-to-MQTT bridge | Consume configured bridge telemetry via existing MQTT boundaries. Requires a supported physical coordinator and joined devices; not installed by this task. |
| [OpenThread Border Router](https://github.com/openthread/ot-br-posix) | Thread transport/controller infrastructure | Integrate configured border-router observations; software cannot supply a missing radio/controller. |
| [Zeek](https://github.com/zeek/zeek) | Network analysis telemetry | Optional external sensor ingestion; not embedded as a second control plane. Physical packet visibility remains necessary. |
| [Suricata](https://github.com/OISF/suricata) | IDS/network telemetry | Optional bounded EVE observation ingestion; no automatic IPS/firewall mutation. Keep license/process boundaries explicit. |
| [surge-ping](https://github.com/kolapapa/surge-ping) | Async ICMP | Not a replacement for native discovery; raw socket privileges differ by OS. Do not claim that adding the crate grants packet access. |

Upstream descriptions are capability claims, not Archie hardware validation. No
third-party repo is executed blindly or allowed to enroll/control devices.

## Implementation design

- Backend choices: native, mdns-sd, hybrid. Native stays available. The supplemental
  backend is bounded by the same scan window and a maximum of 64 service types.
  Select the local interface, drop public/foreign-scope addresses, bound fields and
  report daemon/socket errors. The selected 0.21.5 release silently logs some
  bind/join failures rather than emitting `DaemonEvent::Error`; constructor success
  or `IpAdd` alone is not availability proof. Archie marks reception unconfirmed
  without a valid resolution, and distinguishes interface registration from
  IPv6 multicast transport coverage. Advertised A/AAAA addresses are service claims, not
  proof of an authenticated transport peer; do not derive actuator endpoints from
  them or auto-print. Link-local IPv6 without scope is not an HTTP target.
- Autonomous watch preserves a bounded private history (4096 devices, file <=8 MiB).
  Last response and last cache observation are separate. Historical entries do not
  carry old power/app states as current and never refresh Core peer timestamps or
  feed live DeviceSeen events. Current ARP metadata retains explicit cache provenance;
  a changed MAC on the same IP drops the old label. Candidate ownership stays unknown.
  Internal response clocks stay in the private history and stale-row annotations;
  unchanged live states do not emit artificial feeder changes on every poll.
- Light passes and deep passes have separate clocks. Default light cadence 30
  seconds, deep cadence 600 seconds, TCP budget 75 seconds. A light pass runs the
  native listeners only; the deep pass adds the TCP sweep and the `mdns-sd` daemon.
  What the deep sources (`tcp-presence`, `mdns-sd`) saw stands until the next deep
  pass: a light pass neither drops it nor makes a device or its fields flicker. Only
  identity and those sources' own fields carry over, never power/app/session state.
- A local network wider than /24 is swept only in the /24 around this node.
- History retention runs from the last answer, the last cache sighting or the first
  appearance, whichever is latest; a device in the current inventory is never pruned.
  The device limit evicts records not seen in this pass, oldest first.
- A clock that steps back is handled: recorded times ahead of now are read as now,
  and a deep pass recorded more than one interval in the future no longer holds
  deep passes back until wall time catches up.
- The neighbour table's reachability state (REACHABLE/STALE/DELAY) is not a device
  fact; it changes by itself every few seconds. `seen_by` is reported sorted.
- The native passive mDNS listener binds the group address 224.0.0.251:5353, so it
  receives only group traffic and unicast for 5353 still reaches the system's
  responder (avahi, systemd-resolved). On Windows it is not opened.
- DNS-SD lists a host once: by its IPv4 address when it has one in scope, else by
  its lowest IPv6 address (private before link-local).
- Errors preserve the last successful inventory and use exponential retry delay
  from 30 to 300 seconds by default. Publication/validation errors use the same
  backoff; failed health persistence is also visible on stderr. Startup lock/history
  errors fail explicitly. State/health files are private and bounded. A once mode
  permits reproducible bounded runs. No sensor performs pairing, control or installs.

Core policy/leases, existing action adapters, voice interaction and personal/work
context boundaries stay authoritative. This task implements network observation
autonomy, not unrestricted action autonomy or universal physical radio coverage.

User already authorized direct execution and asked to stop requesting permissions.
Design/plan/test/review evidence remain; no additional approval prompt is introduced.

## Reproducible validation

```sh
cargo test --manifest-path core/Cargo.toml -p archie-lan --locked --jobs 1
cargo test --manifest-path core/Cargo.toml -p archie-core --test sentry_discovery_contract --locked --jobs 1
cargo test --manifest-path core/Cargo.toml -p archie-sentry --test feed --locked --jobs 1
cargo clippy --manifest-path core/Cargo.toml -p archie-lan --all-targets --no-deps --locked --jobs 1 -- -D warnings
archie-lan watch --autonomous --once --seconds 12 --probe-local --out PRIVATE_DIR/lan.json --diagnostics PRIVATE_DIR/coverage.json
# Repeat exactly the same command before 600 seconds elapse; presence should be null.
```

The owner-PC run completed all 3795 sampled TCP attempts, reported zero monitor
failures and resolved services through the supplemental backend. Seven address
records included IPv4/IPv6 aliases of Roku and Spotify and two cache-only candidates;
they are not seven uniquely identified devices. Restarting from the same history
preserved the deep timestamp and skipped TCP while refreshing light observations.
No printer/mobile ownership was established and no print job was sent. Raw network
identifiers remain in the owner's private temporary evidence directory.

IPv6 conversion fixtures and advertised AAAA observations do not validate actual
IPv6 multicast transport, real bulbs/radios, pairing or deployment on other OSes.
BLE/Thread/Zeek/Suricata entries above are researched follow-ups, not new completed
integrations. Existing MQTT/BlueZ/Matter adapters remain in place.
