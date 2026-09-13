# Software worth shipping with ArchieOS

Surveyed 2026-09-13 for the Arch Linux based WSL2 image. Repository, version, licence and
installed size were read from the archlinux.org package API and the AUR RPC on that date.

**Installed size is the package alone, without dependencies.** `pandoc-cli` is 0.2 MiB and
pulls a large Haskell library tree; `ffmpeg` and the GUI applications pull hundreds of MB.
Measure the image, not this table.

## The selection rule

ArchieOS is C++ and Rust, and nothing in it is Python. Archie also never *clicks* a tool:
a model proposes, policy grants a scoped capability, and the executor runs the tool inside
an authorised workspace. So a tool earns a place by its **automation surface** — a CLI with
file inputs and outputs, or a C, C++ or Rust library — and by how risky its actions are.

A tool whose only practical automation API is Python cannot be driven without writing
Python, and is marked as such rather than quietly proposed with glue.

Risk classes follow the capability model: `read_only`, `workspace_write`, `network_egress`,
`device_access`, `system_admin`.

## Base image

Small, useful to almost every professional, automatable from a CLI or native library.

| Package | Repo | MiB | Licence | Automation surface | Risk | Why |
|---|---|---|---|---|---|---|
| `bubblewrap` | extra | 0.1 | LGPL-2.1+ | unprivileged namespaces, bind mounts | system_admin (executor only) | The process boundary the architecture currently lacks: run every tool with only its workspace mounted and no network unless granted. |
| `nsjail` | extra | 0.7 | Apache-2.0 | namespaces + seccomp + cgroup limits | system_admin (executor only) | Alternative jail with resource limits built in. Pick one of the two. |
| `age` | extra | 12.6 | BSD-3-Clause | CLI encryption | workspace_write | Encrypted exports and backups of vault material. |
| `restic` | extra | 41.4 | BSD | CLI, encrypted, deduplicated | workspace_write / network_egress | Backups of workspaces and sealed vault entries. |
| `ripgrep`, `fd`, `jq` | extra | 4.3, 3.4, 0.5 | MIT/Unlicense, MIT/Apache, MIT | CLI | read_only | Workspace search and inspection capabilities. |
| `qpdf`, `poppler` | extra | 3.5, 6.7 | Apache-2.0, GPL/LGPL | CLI + C++ library | read_only | PDF inspection, text extraction, splitting. |
| `tesseract` + `tesseract-data-spa` | extra | 4.7 + 17.4 | Apache-2.0 | CLI + C++ library | read_only | OCR, with Spanish. |
| `typst` | extra | 45.4 | Apache-2.0 | single Rust binary | workspace_write | Document generation from text. |
| `sqlite`, `duckdb` | core, extra | 16.8, 85.8 | public domain, MIT | CLI + C API | workspace_write | Local structured data and analytics without a server. |
| `openscad` | extra | 8.3 | GPL-2.0+ | `openscad -o part.stl part.scad` | workspace_write | Parametric CAD as plain text and a CLI — CAD with no Python anywhere in the loop. |
| `ngspice` | extra | 18.5 | BSD-3-Clause | CLI + shared library | workspace_write | Circuit simulation. |
| `mosquitto` | extra | 1.8 | EPL-2.0 | broker + CLI clients | device_access | MQTT for Sentry's device fabric. |
| `avahi` | extra | 2.0 | LGPL-2.1+ | mDNS | network_egress | Local device and fabric discovery. |
| `onnxruntime-cpu` | extra | 25.7 | — | C/C++ inference API | read_only | Small local models: the floor estimator, PII detection, embeddings. |
| `espeak-ng` | extra | 18.1 | GPL-3.0+ | CLI + C library | device_access (audio) | A Spanish-capable TTS fallback. Robotic, tiny, and better than no voice. |

## Offered at first run

Large, GPU-dependent, GUI-first, or with a risk class that needs an explicit policy first.

| Package | Repo | MiB | Licence | Automation surface | Note |
|---|---|---|---|---|---|
| `kicad` | extra | 190.9 | GPL-3.0+ | `kicad-cli` (ERC/DRC, exports) | Electronics without scripting. |
| `prusa-slicer` | extra | 180.6 | AGPL-3.0 | CLI slicing to G-code | AGPL: keep it an unmodified separate program. |
| `opencascade` | extra | 155.6 | LGPL-2.1+ w/ exception | C++ B-rep kernel | FreeCAD's own kernel. The C++ route to real STEP geometry without Python. |
| `freecad` | extra | 301.2 | LGPL-2.0 | **Python macros / `freecadcmd` only** | See the note below. |
| `gdal`, `postgis` | extra | 41.9, 46.6 | MIT, GPL-2.0+ | CLI + C API | ArchieOS already runs PostgreSQL and ingests OSM. |
| `opencv` | extra | 112.8 | Apache-2.0 | C++ library | Local vision. |
| `inkscape`, `libreoffice-fresh` | extra | 190.3, 423.1 | GPL, LGPL/MPL | CLI export, headless conversion | Useful, heavy. |
| `ollama` / `ollama-cuda` | extra | 67.1 / 998.3 | MIT | HTTP API | Local models behind the router. The CUDA build only on NVIDIA machines. |
| `onnxruntime-cuda` | extra | 618.7 | — | C/C++ API | GPU variant. |
| `podman` | extra | 82.3 | Apache-2.0 | rootless containers | Confines external harnesses and risky tools. |
| `gopass` | extra | 20.8 | MIT | CLI secret store | Headless-friendly for WSL; `keepassxc` (31.0) if a GUI secret service is wanted. |
| `arduino-cli` | extra | 25.6 | GPL-3.0 | CLI | device_access — needs a policy. |
| `nmap`, `wireshark-cli` | extra | 26.3, 100.3 | NPSL, GPL-2.0 | CLI | network_egress, and capture is privileged. Never preload. |

## Build pinned in the image pipeline

| Package | Where | Licence | Why |
|---|---|---|---|
| llama.cpp | only `llama.cpp-git` variants in AUR | MIT | A floating `-git` package is not reproducible. Build a pinned release from source in the image build; it is C++. |
| `piper-tts` | AUR | GPL-3.0+ | Far better Spanish TTS than espeak-ng. Invoke as a separate program. |
| `sherpa-onnx` | AUR | Apache-2.0 | C++ runtime for ASR, TTS and VAD on ONNX. |
| `osmium-tool`, `tippecanoe` | AUR | GPL-3.0, BSD | C++ OSM and vector-tile tooling — directly useful for the Geofabrik pipeline card. |
| `gmsh`, `calculix-ccx`, `orca-slicer` | AUR | GPL-2.0+, GPL, AGPL-3.0 | Meshing, FEM, slicing. |
| `landrun` | AUR | MIT | Landlock-based confinement without root. |

## Not recommended

| Package | Reason |
|---|---|
| `blender` (385), `qgis` (436), `meshlab` (AUR) | Automation is Python (`bpy`, PyQGIS, pymeshlab). |
| `esptool`, `platformio-core` | Python tools. `arduino-cli` covers the common case. |
| vLLM | Python. |
| `zigbee2mqtt` (257.4) | A Node.js service; defer until a device actually needs Zigbee. |
| `firejail` | SUID root. `bubblewrap` or `nsjail` give confinement with a smaller attack surface. |

## FreeCAD needs a decision

The workstation plan makes FreeCAD the first professional tool, with `freecad.run_python`
among its calls. FreeCAD is C++, but the only practical way to drive it is a Python macro
executed by `freecadcmd`. That collides with the project rule.

Three honest options:

1. **Keep FreeCAD, confine the Python to FreeCAD.** Archie's adapter stays Rust and passes
   FreeCAD a *fixed, reviewed* macro set shipped as data, never model-written Python.
   `run_python` with model-supplied code is removed. This is the smallest change.
2. **OpenSCAD for the parametric demo.** Text in, STL out, no Python anywhere. It cannot
   produce STEP.
3. **OpenCASCADE from C++.** The same kernel FreeCAD uses, called directly. STEP-capable
   and fully in-language, at the cost of writing the modelling layer.

For the hackathon demo, option 2 is the fastest way to a parametric part that prints. For
engineering use, option 3 is where this ends up.

## Licensing when shipping

Distributing GPL and AGPL programs alongside Apache-2.0 Archie code is aggregation, provided
Archie invokes them as separate programs and does not link them. Do not link GPL libraries
into Archie binaries. LGPL libraries with the OpenCASCADE exception can be linked
dynamically. AGPL tools (`prusa-slicer`, `orca-slicer`) carry obligations if modified and
offered over a network.
