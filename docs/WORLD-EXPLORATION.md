# Native World exploration

GeoFabric plans bounded geographic work in Rust; `archie-worldd` fetches the data,
keeps checkpoints and exports, and publishes normalized previews. World draws the
real terrain/building samples through the existing native renderer and the shared
Xavi/PentaVM shell theme. Aleph source/port provenance is in
[`third_party/aleph`](../third_party/aleph/README.md).

## Local owner session

Exploration is explicitly enabled. The stock isolated `DynamicUser` world-feed
service remains unchanged; it is not an exploration deployment recipe. Core,
worldd and displayd need an intentionally configured owner session/private runtime
for the commands below. Do not start a second process over an occupied 7667 port.

```sh
cd core
cargo build --locked -p archie-worldd -p archie-cli -p archie-displayd
export ARCHIE_WORLD_RUNTIME_DIR="$XDG_RUNTIME_DIR/archie-worldd"
target/debug/archie-worldd --enable-exploration --domain personal
# In another terminal with the same runtime environment:
target/debug/archie world plan --latitude 41.8902 --longitude 12.4922 \
  --radius-m 50 --zoom 14 --layers terrain --domain personal --json
target/debug/archie world explore --latitude 41.8902 --longitude 12.4922 \
  --radius-m 50 --zoom 14 --layers terrain --domain personal --json
target/debug/archie world status --domain personal --json
target/debug/archie world resolve 'Colosseum, Rome' --domain personal --json
target/debug/archie world pause CAPTURE_ID --domain personal
target/debug/archie world resume CAPTURE_ID --domain personal
target/debug/archie world cancel CAPTURE_ID --domain personal
target/debug/archie visual scene world
```

`--max-tiles` defaults to 256; `--max-bytes` defaults to 64 MiB and bounds both
downloaded application payload and stored capture data, including cache copies
and exports. Terrain supports source zoom 0..14. `--layers terrain,buildings`
combines the two supported native capture layers. The CLI requires an explicit
personal/work domain and acknowledges acceptance separately from completion.

The private command descriptor is **not** passed to displayd or Qt. A separate
owner-private `projection.json` grants only GET access to the exploration view;
both `/v1/exploration` and `/api/exploration` require its bearer credential. The
native client reads this file without exposing it to QML. A Windows native view
over WSL needs a matching explicit runtime path accessible to that owner; no
automatic cross-identity credential sharing is installed by this feature.

## Voice authority

The existing `live` voice mode accepts `--world-domain personal/owner` or a full
`work/organization/project` scope. This trusted host setting and the existing
session privacy grade accompany proposals to Core. The model cannot widen them.
Core verifies the kernel peer owner, operator profile, exact context scope,
privacy ceiling, emergency Policy state and one-operation leases for fixed
provider hosts. External providers accept explicitly Public data only. Core
rechecks authority after the reply; scoped status excludes other projects,
unbound owner-CLI jobs and global provider aggregates. Windows cannot authenticate
the Unix operator peer and refuses this voice gateway there.

## Concurrency and limits

Foreground app, keyboard/mouse activity outside World, window exposure and visual
render tiers are not download scheduler inputs. Manual World navigation only
cedes its camera. Jobs continue while another app is used or the viewer closes.
Explicit job pause/cancel, configured byte/tile limits, provider admission and
provider failures can stop/wait a job. There is no OS packet shaper or global
human-input suspension added by this feature.

Provider admission is shared across jobs in the daemon. Defaults are **local
caps**, not claims about external quota: terrain 120 requests/minute and 4 in
flight; OSM 6/minute and 1; Photon 30/minute and 1. HTTP `Retry-After` supports
seconds and dates; reported quota remains distinct from local policy. Concurrent
requests for the same resource share a cache lock. The source cache has a 128 MiB
and 2048-file bound with eviction restricted to generated SHA256 names.

Manifests/checkpoints survive interruption. A daemon restart leaves unfinished
jobs paused for explicit resume; preserved source files are reused before making
new requests. Empty coverage completes with empty GeoJSON, without inventing a
GLB. UI projections omit old previews and bound current geometry; omitted
buildings/truncated metadata are reported while complete files remain on disk.

## Truth and supported scope

- Native sources: public AWS/Tilezen terrain GeoTIFF, closed OSM building ways,
  and Photon lookup. Source credits/permissions remain separate from code license.
- Original TIFF/OSM captures and GeoJSON/GLB exports are kept. The displayed/exported
  terrain mosaic is a bounded 33×33 Web Mercator preview, not a full-resolution DEM.
- Source nodata stays missing. Building height is an explicit OSM tag or unknown;
  building bases use a labeled 0 m preview assumption. Vertical datum is unverified.
- Pure planning handles the antimeridian, but capture/export currently asks for
  two non-wrapping areas before any requests. Polar Mercator regions are refused.
- Multipolygon/hole buildings, Google satellite/Street View/Earth extraction,
  textured imagery and photogrammetry are not advertised as implemented. Existing
  GeoFabric storage/analysis restrictions are preserved.
- HUD effective TPS is the latest measured Core `AiInfer` output-token count over
  its full execution time, not generation-only throughput or all voice traffic.
  Unreported/stale measurements remain unavailable; no benchmark is displayed as live.
- Tests, real network smoke and limitations are recorded in
  [`_dev/WORLD-GEOFABRIC-RUST-2026-10-24.md`](../_dev/WORLD-GEOFABRIC-RUST-2026-10-24.md).
  Headless tests do not verify GPU behavior, live voice hardware or packaged deployment.
