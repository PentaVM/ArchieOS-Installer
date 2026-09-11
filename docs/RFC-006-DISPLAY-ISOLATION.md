# RFC-006 — Display isolation and the observatory protocol

- **Status:** accepted, implemented through ArchISO packaging (Wave E / E0–E2);
  physical GPU/seat verification remains open
- **Supersedes:** nothing. The v0.4 PySide6 desktop app on `legacy-v0.4` is reference
  material, not an ancestor of this design.

## 1. Problem

The ArchieOS server has a monitor attached. That monitor is not a desktop. It is an
always-on cognitive observatory: it shows what Archie is actually doing, and it must be
readable from across a room while exposing dense evidence up close.

Two things must be true at once:

1. The display must be **beautiful and information-dense**.
2. The display must be **incapable of harming the control plane**.

A visualization that shares a process with `archied` fails (2). A visualization that
scrapes `archie status` text fails (1) and breaks on the first formatting change.

## 2. Decision

Insert an unprivileged bridge between the trusted daemon and the renderer.

```text
Linux kernel
→ systemd
→ archied                trusted control plane        user: archie
→ control protocol       newline-delimited JSON over a Unix socket
→ archie-displayd        read-only bridge             user: archie-display
                    ◀── normalized JSON on loopback ── archie-worldd (DynamicUser)
                                                      fixed USGS HTTPS adapter
→ bounded JSON on loopback
→ archie-display         native C++ / Qt Quick 3D     user: archie-display
→ Qt EGLFS → DRM/KMS     direct fullscreen GPU output; no web runtime/compositor
```

Four properties follow from that shape, and each one is the reason a layer exists:

**The display cannot mutate anything.** `archie-displayd` speaks only the read-only
subset of the control protocol. `ControlRequest::is_mutating()` already exists in the
protocol and returns `false` for every v0.1 request; displayd asserts on it before
forwarding, so adding a mutating request later cannot silently become reachable from the
native graphics client.

**The display can crash without consequence.** It is a separate process under a separate
account. If it dies, `archied`, the fabric, research and the physical safety boundary
continue untouched, and systemd restarts it.

**The renderer never holds a secret.** Provider credentials stay outside the renderer.
The native process receives normalized records only.

**The renderer never invents state.** Every snapshot carries a `source` field —
`live` or `fixture`. The UI renders that distinction prominently. A recorded fixture is
a legitimate development tool; a fixture that *looks* live is a lie, and this field is
what prevents one becoming the other.

## 3. Why not the alternatives

**Render inside `archied` or the Linux kernel.** Rejected: it would put a GPU/UI stack
inside an authority boundary, and a shader compiler or scene-graph crash would take
down physical safety coordination.

**Scrape the `archie` CLI.** Rejected: human-readable output is a presentation format,
not an interface. It changes for readability reasons and would break the display each
time, and it cannot carry structure the tables omit.

**Expose `archied`'s socket to the renderer directly.** Rejected: the control socket is
group-gated on the `archie` account. Handing that group to a renderer would give the
graphics process the same reach as the CLI.

**Browser kiosk / Electron / embedded web view.** Rejected: the observatory is a system
instrument, not a web application. A browser adds a second process model, HTML/JS,
profile/cache state and a much larger idle memory and attack surface. The native Qt
Quick 3D scene is compiled into one ELF and uses the Qt scene graph directly.

**A desktop environment.** Rejected outright. GNOME/KDE bring a session manager, a
compositor, a file manager, a notification daemon and a login greeter — an enormous
attack surface and a boot that flashes a desktop before showing Archie. EGLFS runs the
single native client directly on DRM/KMS without a desktop or compositor.

## 4. The snapshot protocol

`archie-displayd` exposes three machine routes on loopback only. It serves no HTML,
JavaScript, texture or general file route:

| Route | Returns |
|---|---|
| `GET /api/snapshot` | `DisplaySnapshot` as JSON |
| `GET /api/world` | normalized `WorldSnapshot` observations and provider health |
| `GET /api/kernel` | machine telemetry read locally from `/proc` and `/sys` |
| `GET /healthz` | liveness, including whether `archied` is reachable |

`/api/kernel` is deliberately not folded into `/api/snapshot`. Everything in the snapshot
comes from `archied` over the control socket, so when the daemon is unreachable the whole
projection is correctly marked stale. Pressure and thermals do not come from `archied` —
`archie-displayd` reads them itself, on this machine — and folding them in would discard a
genuinely useful property: **when the daemon is down, this is still true.** An operator
staring at a disconnected observatory most wants to know whether the box is thrashing or
overheating, and that is exactly the moment the daemon cannot say.

Two rules govern how it renders:

- **Unmeasured and idle never look the same.** PSI needs `CONFIG_PSI=y`; thermal zones
  need a driver that publishes them. When either is missing the projection carries
  `Unavailable { reason }` and the view draws a dashed rule, not a bar at zero. A gauge
  reading zero says the machine is quiet; a gauge that cannot read says nothing is being
  measured, and an operator acts differently on each.
- **Each thermal bar is scaled against its own zone's critical trip.** A CPU at 90 °C
  rated to 100 and an NVMe at 90 °C rated to 85 are not in the same condition, and one bar
  on one shared axis would claim they are. A zone whose firmware declares no critical trip
  has no scale at all, so its bar refuses to draw rather than inventing one.

The `display_budget` field is the one the renderer acts on: it selects the antialiasing
mode and quality, so a saturated machine visibly gets its GPU back instead of the
observatory merely reporting that it is busy. It is a serialized field rather than a
method because the renderer reads it over HTTP, and it is private on the Rust side so that
only the read path can set it — a budget that disagreed with the verdict printed beside it
would be worse than no budget at all. Unknown pressure yields `balanced`, never
`cinematic`: absent evidence, a display must not assume it has room.

`DisplaySnapshot` is one typed aggregate rather than five separate polls, so the renderer
never draws a frame mixing a fresh module list with a stale topology.

It composes the existing control responses:

```text
DisplaySnapshot
├── source          live | fixture        ← honesty marker, never omitted
├── generated_at
├── daemon_reachable
├── status          StatusView            identity, hardware, capabilities, counts
├── boot            BootReport            the canonical boot sequence and its verdicts
├── topology        capability index, nodes, totals
├── modules         ModuleView[]          state AND the reason for that state
├── load_order      dependency-resolved order
├── models          ModelView[]
└── metrics         counters and gauges
```

When `archied` is unreachable, displayd does **not** serve the last good snapshot as if
it were current. It serves `daemon_reachable: false` with the stale data and its
timestamp, and the renderer visibly degrades. Pretty visuals must not hide failure.

World data has an independent process, contract and refresh task. `archie-worldd` reads the
no-credential USGS earthquake GeoJSON feed over HTTPS with a two-second connection
timeout, four-second total timeout and a two-MiB response cap. Provider JSON is decoded
into typed positions, earthquake values and provenance inside that isolated adapter; raw
provider objects and URLs never cross its loopback API. Failed requests keep the
last-known-good batch, mark it stale, and use bounded exponential backoff. With no cache,
the provider is explicitly `degraded` and the observation list is empty. The service can
also start with `--disable-world-feed`, in which case local observability is unchanged
and `/api/world` says `disabled`.

`archie-worldd` is the only display process with Internet egress. It runs as a dynamic
user, has no `archie` group, cannot use `AF_UNIX`, and has Archie configuration, runtime,
state and logs explicitly hidden. Its provider URL is a compile-time constant: there is
no URL argument, credential input, generic proxy route or raw-response route.
`archie-displayd` retains `IPAddressDeny=any`/`IPAddressAllow=localhost`; it contains no
Internet HTTP client and only reads the normalized contract from `127.0.0.1:7667`.

## 5. Visual semantics

No major visual element is ornamental. The mapping is fixed and documented so a viewer
can read the screen without a legend:

| Channel | Encodes |
|---|---|
| ring | architectural layer: core → runtime → agents → resources |
| angle | stable per identity, so a module keeps its position between frames |
| size | relative importance within its layer |
| brightness | activity and freshness |
| colour | semantic state, from the design tokens |
| opacity | confidence / staleness |
| dimmed + outside the rings | installed but not loaded, with the reason attached |
| Earth marker position | provider-reported latitude/longitude |
| Earth marker size | provider-reported magnitude |
| event pulse | a changed typed snapshot, never an idle timer |
| loading bar | an actual in-flight bridge request |

That last row matters most. A module that failed to load is drawn, not hidden. The
0.1.0 ISO shipped a daemon that could not read its own config; a display that showed
only healthy components would have rendered an empty, serene screen while the system was
dead.

## 6. Native session

```text
systemd-logind
→ archie-display.service      user: archie-display
→ archie-display              compiled C++ / Qt Quick 3D
→ Qt EGLFS / eglfs_kms
→ DRM/KMS + OpenGL
```

The ISO contains no Cage, Chromium, Electron, HTML or JavaScript application. QML is a
Qt-native declarative scene description compiled into the ELF by `qt_add_qml_module`;
the C++ bridge owns network parsing, topology construction, positions, colors and real
revision signals. The NASA texture is a checksummed ELF resource. EGLFS acquires the
dedicated tty/DRM device directly, so there is no window manager or compositor to fail.

## 7. Resource governor

The display must never starve inference or simulation. The current enforceable envelope
is systemd-backed:

```text
archie-display   CPUWeight=20, CPUQuota=100%, MemoryHigh=768M, MemoryMax=1536M
archie-displayd  CPUWeight=40, CPUQuota=25%,  MemoryHigh=64M,  MemoryMax=128M
archie-worldd    CPUWeight=20, CPUQuota=10%,  MemoryHigh=48M,  MemoryMax=96M
```

There is no decorative continuous animation. The globe's slow orientation derives from
UTC; core pulses come from changed snapshot digests; scene transitions come from an
operator action; the loading indicator exists only while a request is in flight. An
adaptive scheduler-to-GPU quality loop remains future work.

## 8. What is implemented now

- `archie-display-protocol` — the `DisplaySnapshot` type and its `source` marker.
- `archie-displayd` — the read-only bridge, its loopback HTTP server, and fixture replay.
- `archie-worldd` — the separately sandboxed fixed-provider adapter; this alone has
  Internet egress and it has no Archie state or control-socket access.
- E2 world contract — normalized/provenanced observations, provider health, bounded USGS
  polling, cache, staleness and backoff semantics.
- The native C++/Qt Quick 3D renderer — satellite globe, real/provenanced observations,
  cognitive topology, hardware, boot sequence and module states with reasons.
- The ArchISO session — a dedicated `archie-display` account, direct EGLFS/KMS, systemd
  resource envelope, tty isolation and a GPU condition. The bridge starts at
  `multi-user.target`; the renderer is a soft dependency of `graphical.target`, so
  display failure cannot block `archied`.

## 9. What is not

- Physical verification of logind, EGLFS/KMS and GPU acceleration on the target server.
  Static packaging contracts and headless behaviour are covered; a Windows development
  host cannot prove DRM-seat acquisition.
- Sentry fusion (E3), research (E4), Nova (E5) and additional cinematic data layers
  (E6).
- The resource-governor feedback loop into the scheduler; the budget is accepted by the
  renderer but nothing lowers it automatically yet.

None of these are stubbed. The seams exist; the features do not pretend to.
