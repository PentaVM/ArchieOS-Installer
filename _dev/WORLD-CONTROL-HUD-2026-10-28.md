# World connection, minimal navigation HUD and operator console

User-directed native implementation, preserving Xavi's design credit and existing
Core/Cortex/IRIS/Orbe/Policy/lease boundaries. Source repository verified:
PentaVM/ArchieOS-Nvidia, existing codex/light-iris-cinema worktree.

## Design and implementation plan

The current left sidebar exposes every diagnostic at once. Replace it with a
floating search box, a small navigation rail and on-demand layers, measurement,
details and CLI panels. Reuse the existing projection/history inspector rather
than replacing source data. Keep imagery attribution visible. Voice remains a
primary interaction; the CLI is an additional explicit operator surface.

- [ ] Repair projection-port propagation into the shell bridge and worldd-port
  selection in displayd. Preserve fail-closed invalid configuration.
- [ ] Repair launcher binary preflight/path wiring, then diagnose real services.
  No fake connected state when Core, PostgreSQL or WSL is absent.
- [ ] Build minimal WorldHud around retained WorldDetails, with coordinate/place
  search, zoom, global home, 2D/3D tilt, imagery layers and geodesic ruler.
  Test ray/sphere picking, geographic axes, antimeridian and malformed input.
- [ ] Add an internal bounded console using fixed QProcess arguments. World
  commands go through the existing typed Core operator gateway with immutable
  host-selected exact ContextDomain and Public privacy. Never use direct worldd
  owner control as a fallback.
- [ ] Route IRIS goals through the existing Stage/Sight operator CLI with trusted
  host configuration. Do not invent app IDs, grant snapshot rights to the whole
  shell, execute /act predictions or bypass preview-only capture restrictions.
- [ ] Test native/CLI/launcher paths, inspect actual light/dark World captures,
  record real connection evidence and remaining environmental limitations.
- [ ] Save a local commit; preserve unrelated LOC reports and other worktrees.

## Root causes established before changes

ShellRuntime omitted --displayd and silently used7666 when native Qt requested
17666. displayd WorldService hardcoded worldd7667 while the existing demo used
17667. The demo did not provide an explicit bridge binary. These are distinct
from GeoFabric planning/provider health.

No local daemons were listening. WSL first failed HCS0x800705aa with0.56GB RAM
free. User authorized closing Windows Firefox; after closure WSL briefly started
successfully and later failed again during compilation pressure. The existing
private PostgreSQL/Core node was found intact but stopped. No database reset,
model deployment or credential disclosure is authorized or needed.

Verification receipts and captures will remain outside Git under
D:/ArchieOS-World-Control-20261028.
