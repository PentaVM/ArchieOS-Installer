Título: feat(gui): Xavi's desktop sphere, native in Rust/C++

Link para crearlo: https://github.com/PentaVM/ArchieOS-Nvidia/compare/main...feat/esfera-neuronal-nativa?expand=1

---

## Summary

The desktop sphere now looks like Xavi's Lumenfield hologram (`docs/gui/screenshots/29-esfera-holograma-frio.png`) instead of an empty orb. Design: Xavi (ByXavi270 / XaviCottier). Native implementation: PentaVM.

- **Content sphere (Rust, `archie-lumen-design::content`)**: bounded, read-only listing of one folder (2 500 nodes, 30 files per folder, hidden entries and build output skipped, links never followed) laid out by `field.js` `buildField` ("regions"); C ABI `design_content_scene` returns the scene `LumenField` already reads.
- **`LumenField`**: setting `fieldSource` (`tfog` | `content`), listing off the GUI thread every 5 s; `--content-root` / `ARCHIE_CONTENT_ROOT`; label anchors for the HUD.
- **Renderer (`archie::visual::lumen`)**: filament curves and the hologram frame (tilted orbit rings with ticks, dotted rim, projector rings and beam, core beam); glow core and haze.
- **`HologramGrade`**: Xavi's CSS filter (hue-rotate 168°, saturate, contrast, brightness) over the finished frame with ACES tone mapping, a light bloom, scan lines and a red/cyan split.
- **Presets** `holograma` (cold, desktop default) and `holograma-calido` (warm). `archie visual preset holograma` works.
- `AGENTS.md`: GUI and sphere credited to Xavi (design) and PentaVM (implementation).

## Verification

- Rust `archie-lumen-design`: 25 tests (4 new). `display/visual`: 19 suites (filament and frame cases added). Native CTest: 4 (new `contentSphereListsItsFolderWithoutTheObserver`). Shell QML: 77.
- Real frames with `--render-check` (WSLg / Mesa llvmpipe): `docs/gui/screenshots/35-…frio.png`, `36-…calido.png`.

## Limits

Lumenfield 0449's own source was not in the delivered archive (it holds Sentry-Graph Astra, 2026-10-09), so rings, filaments and labels are rebuilt from the screenshots, not pixel-identical. Not measured on a hardware GPU. Details: `_dev/verification/xavi/content-sphere.md`.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
