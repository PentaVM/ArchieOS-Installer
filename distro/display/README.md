# Native observatory session

**Status: integrated into the ArchISO; static contracts pass. Physical DRM/KMS
verification remains required.**

The server-room display is a system instrument, not a web app or desktop. The ISO
builds and installs one C++/Qt Quick 3D executable whose QML, shaders and NASA satellite
texture are compiled resources. Qt EGLFS renders directly through DRM/KMS/OpenGL.

```text
archie-worldd.service    DynamicUser; fixed USGS HTTPS; no Archie state
          │ normalized typed loopback JSON
          v
archie-displayd.service  user archie-display; read-only Archie projection; no egress
          │ bounded local JSON
          v
archie-display.service   user archie-display; native C++/Qt Quick 3D → EGLFS/KMS
```

There is no Chromium, Cage, Electron, HTML/JavaScript application, window manager,
desktop environment, login greeter, mutable web root or runtime-downloaded visual asset.

## Authority and failure isolation

- `archie-display` cannot read `/var/lib/archie`, node keys or the control socket.
- `archie-displayd` alone receives `SupplementaryGroups=archie`, uses only read-only
  control requests and has `IPAddressDeny=any` with localhost allowed.
- `archie-worldd` alone has public egress. It is a dynamic user, cannot use `AF_UNIX`,
  cannot see Archie config/runtime/state/logs and fetches one compile-time USGS URL.
- The native renderer may fail or restart without affecting `archied`.
- A missing GPU leaves the system headless and usable on tty1; the display unit does not
  restart-loop because it is conditional on `/dev/dri/card0`.

## Truthful rendering

The Earth view uses an offline, labelled NASA MODIS composite. Every overlay record is
a normalized observation with provider, source record ID, observed/provider timestamps,
coordinates and provider health. If the adapter fails, the last-known-good batch becomes
stale or the layer is empty/degraded. No placeholder point appears.

The CORE view is Archie itself: its center is daemon reachability, nodes are actual boot
stages/modules, edges are boot or manifest dependency relationships, and color is state.
The pulse occurs only when the semantic topology/state digest changes; transport
timestamps and volatile counters cannot trigger it. The loading bar exists only while
a real bridge request is in flight. UTC drives the Earth's slow orientation.

## Resource envelope

The three services have independent systemd CPU, memory, task and file-descriptor
limits. The native renderer is capped at 512 MiB and one CPU; the two Rust projections
have substantially lower limits. The display is a soft dependency of
`graphical.target`, never a prerequisite for the control plane.

## Physical verification still required

CI builds the native executable and boots the same ISO headlessly to `ARCHIE READY`.
The target server must still prove:

1. `archie-displayd` and `archie-worldd` start with their declared accounts/sandboxes.
2. EGLFS obtains tty2 and DRM master through logind without root.
3. The Qt Quick 3D scene renders with hardware acceleration at the target resolution.
4. Keyboard/input cannot escape to a shell or another application.
5. Killing the native renderer leaves `archied` and both data services healthy.
6. Restarting `archied` visibly marks the snapshot stale and then recovers.
7. Removing provider/network access yields zero invented points and an explicit degraded
   state.

Until these pass on the server-room hardware, release notes must say the native display
is included but hardware-unverified.
