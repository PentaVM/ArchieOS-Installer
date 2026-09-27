# Bridges

A bridge is how Archie drives an application by tool rather than by eye. There is one per
application: KiCad, Kdenlive/MLT, FreeCAD, LibreOffice, and so on. Each goes through the
application's own free, official automation surface, and the project maintains it: when
the application changes, the bridge's end-to-end tests say so.

[`bridges/INDEX.md`](bridges/INDEX.md) lists the bridges that exist and how to connect to
them. This page covers how bridges are built and kept working.

## What a bridge is

```text
bridges/<id>/bridge.toml      what it drives, through which surfaces, which programs, how to
                              install them, and when to use the GUI instead
bridges/<id>/SKILL.md         the guide a planner reads before its first call
core/crates/archie-bridges/src/<id>…   the tools, in Rust
core/crates/archie-bridges/tests/<id>_e2e.rs   the same tools against the real application
```

`archie-mcp-server --bridge <id>=<folder>` serves the tools over MCP to any client:

- Archie's own planner;
- Claude Code or Codex;
- an IDE;
- a local model behind a harness.

`archie_bridge_guide` hands out the guide. `archie-bridges call` runs a tool from a shell.

Every tool of a bridge obeys the same rules, in `archie-bridges`:

- **One folder.** The operator names the folder when serving the bridge
  ([`Workspace`](../core/crates/archie-bridges/src/workspace.rs)).
  - Paths in and out are relative to it.
  - Links do not lead out of it.
  - New files never replace old ones. An edit in place keeps the previous version under
    `.archie/history/`.
- **Programs run bare** ([`Run`](../core/crates/archie-bridges/src/run.rs)):
  - a fixed binary with arguments built in Rust, and no shell;
  - an emptied environment, so no secret leaks into the application;
  - a time limit, and capped output.
- **Arguments are data.** Some applications can only be scripted in the language they embed:
  FreeCAD, Blender, GIMP, QGIS, Inkscape. For those
  ([`Script`](../core/crates/archie-bridges/src/script.rs)):
  - the script is a constant in the bridge's Rust;
  - it reads its arguments from a JSON file;
  - nothing a planner sends is formatted into code.

  ArchieOS still carries no Python; the application brings its own interpreter.
- **Grades.** Each tool declares the most sensitive data its answer can carry. The MCP
  server's clearance hides every tool above it.

## Tools first, eyes second

Tools are exact and cheap. The GUI, operated on the Archie Stage and seen by IRIS, can do
anything but is slow. A bridge covers what an application's automation surface does well,
and its manifest says what is left for the GUI (`[vision] when`). Its reads and renders
then check the GUI work: `kicad_drc` after routing, `kicad_render` after placing.

Tool calls made while the application's window is on the Stage are also training data:

- each call is a labelled action;
- the frames before and after it show what the action looked like;
- IRIS learns from those pairs to act in the GUI where no tool reaches.

## Writing a bridge

Follow this procedure in this order, by hand or with any agent. A bridge is listed only
when every step is done.

1. **Find the surfaces.** Read the application's official automation docs. In order of
   preference:
   1. a CLI, or a documented file format;
   2. an IPC, D-Bus or HTTP API;
   3. an embedded scripting language.

   Use only free, official surfaces: no private APIs, no paid add-ons, no scraping of the
   GUI. Record each surface in `bridge.toml` with its documentation URL. List a surface
   you intend to add later as `status = "planned"`.
2. **Scaffold.**

   ```bash
   archie-bridges new <id>
   ```

   This writes `bridges/<id>/bridge.toml` and `SKILL.md` from templates.
3. **Design the tools** around what a person does, not around the API:
   - Name each tool `<id>_<verb>`, such as `kicad_drc` or `video_render`.
   - Take a JSON object, with relative paths.
   - Return JSON, which a planner reads: counts first, then details, capped.
   - Start with reads and checks. Then add writes that make new files. Last, add edits in
     place, which are the riskiest.
   - Give every tool a grade. A tool that returns file names or file contents is
     `internal` at least.
4. **Implement** in `core/crates/archie-bridges/src/<id>.rs`:
   - implement `Bridge`, and add the bridge to `BUILTIN`;
   - use `Workspace`, `Run` and `Script`, never `std::process::Command` directly.
5. **Unit tests**, without the application, for:
   - parsing, and argument checks;
   - refusals: paths outside the folder, bad names, a file open in the application;
   - "nothing was written" when a call fails.
6. **End-to-end tests** in `tests/<id>_e2e.rs`, against the real application. They are
   gated by `ARCHIE_BRIDGE_E2E=<id>` and run on a machine that has it installed. Use the
   application's own sample files where it ships them. A bridge without passing
   end-to-end tests is not listed.
7. **Write `SKILL.md`** for a planner that has never seen the application. It covers:
   - what to call first;
   - the order that works;
   - the traps you hit while writing the tests;
   - when to use the GUI instead.

   A test fails if the guide does not mention every tool.
8. **Index.**

   ```bash
   archie-bridges index --write docs/bridges/INDEX.md
   ```

   Commit the result. A test fails when the index is stale.

## Keeping bridges working

- Each manifest's `e2e` field is the command that runs the bridge's end-to-end tests.
  Run them after an application update, and on a schedule on the test machine.
- A failure is a bridge to fix, not a test to skip. The guide gets the new trap, and the
  manifest's `versions` the range that works.
- An application that changes its automation surface keeps the bridge's tool names and
  answers where it can. A planner that learned the old names should not notice.

## Long jobs and delivery

Work that takes hours runs as a job, not as one tool call:

- The job is a detached `archie-bridges run-job` process ([`jobs`](../core/crates/archie-bridges/src/jobs.rs)).
- It survives the conversation that started it.
- It writes its progress to `.archie/jobs/<id>/`.
- It stops cleanly between iterations when `archie_job_cancel` asks.

A bridge whose job makes stages (`freecad_sweep`, `blender_animation`) delivers each one
to the person's desk as it finishes. [DELIVERY.md](DELIVERY.md) covers the desk, the
tunnel, and which formats each bridge sends for which software.

## Where each bridge stands

| Bridge | Surface | Planned next |
|---|---|---|
| kicad | S-expression files, `kicad-cli` | the IPC API, for live edits in an open board |
| freecad | `FreeCADCmd` + a constant script | TechDraw drawings, assemblies |
| blender | `blender --background` + a constant script | material presets |
| office | flat ODF written in Rust, `soffice --headless` | charts in spreadsheets |
| inkscape | `inkscape --actions`, `--export-type`, SVG | |
| gimp | `gimp-console` batch + a constant script | layer-level operations |
| qgis | `qgis_process --no-python`, GDAL | styled maps from a QGIS project |
| video | MLT `melt`, `ffprobe`, EDL | |
| audio | FFmpeg filters | Audacity mod-script-pipe for open projects |
| obs | obs-websocket 5 | |

Each one's unit tests run everywhere. Its end-to-end tests run where the application is
installed.
