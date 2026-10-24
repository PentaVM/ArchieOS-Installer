# Observatory controls

The native Observatory opens on Orb. Its scene switcher is ordered Orb, Forge, World.
`--scene orb|forge|world` still selects a specific opening view.

Archie's live voice exposes two visual tools: `observatory_scene` and `orb_preset`. The same
controls are on the CLI: `archie visual scene <name>`, `archie visual preset <name>` and
`archie visual list`. Both are clients of `archie_orb::control`, which sends one JSON line to
the native display on `127.0.0.1:7668` and reads one line back (`ok` or `error: <reason>`).

The display is the only place the names live. `list` answers with its scenes (`orb`, `forge`,
`world`, from `DisplayControl`) and its presets: every `presets/*.json` file is embedded in the
binary and is a preset named after its file. `field` (Regiones vivas) is the original look,
with no overrides. Applying a preset opens Orb. A preset may only set keys that exist in the
renderer's defaults, with values that pass the same checks as the panel (type, `#rrggbb`
colours, known layout, numbers inside their range, not clamped): one bad key refuses the whole
file and the reply says which. Broken preset files are reported when the display starts. HUD
labels are the `labels` setting, so a preset can hide them. Local overlay toggles use
the same presentation settings and do not change OS authority.

PR41 also preserves its local `cinematic`, `overlay` and shell `panel` controls. The
eight `polar-*` profiles from Xavi's delivered source are embedded by the Rust
`archie-lumen-design` static library and join the same validated renderer catalog.
Qt/QML, GPU shaders and World remain the native host. See
[`source-port.md`](../../_dev/verification/xavi/source-port.md) for source attribution,
CPU validation and the explicit visual/hardware fidelity limits.

`holograma` and `neural` are Xavi's desktop sphere (Lumenfield's "Esfera neuronal"): Rust
lists a folder read-only and lays the sphere out (`fieldSource: neural`, the
`archie-lumen-design` crate's `neural_*` ABI); Qt Quick 3D draws it with his points, link
layers, sparks, projection base and cone, bloom and hologram pass, and labels it in Inter.
`--preset holograma --content-root <dir>` opens it. The older content source
(`fieldSource: content`) stays in the renderer for the `field`/`crown`/`atlas`/`dense`
profiles; no preset of Xavi's uses it any more.

The loopback channel also takes the verbs an AI needs to see what it does, each checked before
it moves (`archie visual focus|camera|window|pointer|highlight|popup|freeze|capture`); see
`DisplayControl.h` for the wire format. `capture` writes a PNG into the display's own folder
(`$XDG_RUNTIME_DIR/archie-display`) and replies with its path.

Build with Cargo plus the existing Qt6/CMake dependencies; CMake builds/links the
Rust static library automatically. Optional CPU shader checks use Mesa llvmpipe:

```sh
cmake -S display/native -B build/native -G Ninja -DARCHIE_NATIVE_BUILD_TESTS=ON -DARCHIE_NATIVE_BUILD_SHADER_TESTS=ON
cmake --build build/native --parallel 2
QT_QPA_PLATFORM=offscreen QT_QUICK_BACKEND=software ctest --test-dir build/native --output-on-failure
```

The shader test explicitly selects/verifies software OpenGL itself. It is a synthetic
rendering test, not a benchmark. ISO/VBox keep their existing launch contract. For a
WSL Operator image, use that build as `ARCHIE_NATIVE_BUILD_DIR=build/native` when
packaging; `archie-design --preview` is the explicit synthetic QML preview.

The listener changes only the local view. It cannot run commands, access Stage,
or write to displayd. If the Observatory is closed, Archie reports that the local
window did not respond.

Forge includes a CLI tile that starts `foot` when a graphical Wayland or X11
session and `foot` are present. The EGLFS image has no window manager, so Forge
shows its existing launch limitation there.
