# ArchieOS installation wizard

Native Qt Quick UI, built independently from the observatory. Eight pages cover
node/network inventory, owner preferences, declared node location, mounted storage,
regional map packages, sensor checks, mobile passkeys and a final review.

Build with Qt 6 Quick, QuickControls2, Network and Multimedia:

```sh
cmake -S display/installer -B build/wizard -G Ninja -DCMAKE_BUILD_TYPE=Release
cmake --build build/wizard
```

On the live UEFI USB, select **Install ArchieOS - graphical wizard**. Alternatively
run `archie-wizard-session` from its local console. The session starts a temporary
Weston compositor for Qt, GParted and the terminal installer. The installed
observatory retains its EGLFS service; live-only packages are removed from the
target before initramfs generation. The installer uses a whole target disk and
requires explicit confirmation of that disk. It does not preserve partitions on
the selected system disk. An optional map partition must already be mounted,
ext4/btrfs, and on another physical disk. The live source disk is excluded.

The private `wizard-profile.json` contains preferences, optional coordinates and
sensor consent; never passwords, recordings, templates or successful enrolment
claims. Restarting restores preferences, while location confirmation and sensor
consent must be renewed. The Rust installer validates the bounded, versioned profile
before any disk operation. Owner aliases do not grant fabric membership.

Map regions come from the [Geofabrik catalog](https://download.geofabrik.de/technical.html).
The native `archie-map-fetch` supports verified offline cache hits, HTTP range resume,
configured HTTPS mirror failover, byte-count checks and MD5 against the authoritative
source over HTTPS. It also records SHA-256 and attribution in a local manifest.
An OS advisory lock prevents concurrent publication; rejected bytes are quarantined.
Interrupted publication has a recovery journal; ambiguous journals require inspection.
PBF is the compressed source package. Rendering streets/buildings still requires a
separate local index and tile renderer. No billed map API is used.

Camera preview measures image brightness, and the microphone measures peak level.
Neither identifies a person. Optional mobile approval is checked through an
authenticated, replay-resistant local channel before activating either sensor.
See `tools/owner-companion/README.md` for the separately configured passkey service.
It is currently a local development component, not an automatically provisioned
ISO service. Generic Core permission execution and biometric template enrolment
are not implemented by these sensor checks.

For visual QA, `archie-wizard --render-check /absolute/new.png --render-page 6`
captures an actual Qt frame and exits. It refuses to overwrite an existing image
and does not enable a camera or microphone.
