# Lumenfield port: Xavi's Sentry Graph Astra on the ArchieOS stack

This is the record for the port of Xavi's "Sentry Graph — edición Astra" (internal name ARCHIE Lumenfield)
to Rust and C++/Qt. The design is Xavi's; the implementation is PentaVM's.

- Branch: `claude/lumenfield-port`, the only branch for the port. It belongs to a PR towards `claude/unified-0.1`.
- Base: `d5bf2c8`, the merge of the native content sphere `e187986` over unified `1d7f1c2`.
- Reference source: the owner's local folder `codigo fuente`. It is read-only and outside git, and none of its
  Go, Electron or JS runtime is imported.

## Owner decisions (2026-10-22/23)

- The 3D scene must look identical to the original. The original is rendered in Chrome on the CPU with
  SwiftShader; the port is captured with Qt offscreen on llvmpipe. Accepted tolerance: SSIM ≥ 0.98 for the
  scene and ±1 px for control geometry.
- Xavi's HTML panel UI is not rebuilt as a HUD. Its options and presets move to the backend and the CLI, so
  the AI can drive them.
- The port targets Linux only (VM or WSL) and uses Inter as the font.
- The 4K PNG maps are replaced in git by a generator that reproduces them exactly: the sha256 of the decoded
  pixels must match.
- Keep presets and the PR #40 scene and preset CLI. Add verbs for focus, camera, window, pointer, highlight
  and popup, so the AI's live activity is visible. The display stays presentation only and has no OS authority.
- Every button in the OS GUIs must work.
- The connection indicator opens a settings panel in the OS design. It covers IRIS video, camera input,
  microphone and the Archie voice connection. The voice hook is authorized.

## Log

- 2026-10-23: re-cut onto `d5bf2c8` with the owner's OK. Validated the base in WSL:
  - `cargo test -p archie-lumen-design`: 25/25 pass.
  - `display/native`: cmake and ninja build cleanly. The only warning is the `QImage::mirrored` deprecation in
    `XaviDesignBridge.cpp:166`.
  - ctest offscreen (llvmpipe): 4/4 pass (native, shell_runtime, display_control and cpu_shader contracts).
- 2026-10-23 (port phase 1 start, base `5b1864d`): the record is committed. Baseline re-captured in WSL:
  `cargo test -p archie-lumen-design --locked --offline` 25/25 pass
  (`/var/tmp/archie-logs/port1-base-lumen.log`).
- 2026-10-24: phase 1 is committed on `claude/unified-0.1` (`ab70d15`). The native LumenField takes every
  profile, and the legacy ones, from the `archie-lumen-design` catalog. Fixtures pin the catalog in the crate's
  tests and in `test_native`. One value stays on the OS's budget instead of the original's: `visibleLimit`,
  12..420 nodes against the original's 2400. The test says so. ctest offscreen 4/4, QML 84/84. At 10:00 there
  was no newer orb from Xavi (no `design/lumenfield-0449-orbe` branch), so this is the orb ArchieOS ships.
