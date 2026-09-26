---
name: bridge-kicad
description: Work on KiCad schematics and PCBs through Archie's kicad bridge — read a design, run ERC/DRC, fix part fields, export gerbers/drill/BOM/STEP, render the board. Use when a task involves a .kicad_pro, .kicad_sch or .kicad_pcb.
---

# KiCad through Archie

The bridge works in one folder of KiCad projects. Every path you pass and get back is
relative to that folder. Nothing you send is run as code; the bridge reads KiCad's files
itself and runs `kicad-cli`, KiCad's own command line, for checks and exports.

## Before the first call

1. `kicad_version`. If it fails, KiCad is not installed where the bridge looks. On
   ArchieOS: `pacman -S kicad kicad-library`. Say so and stop; do not guess results.
2. `kicad_projects` to find the project, its root schematic and its board.

## Understanding a design

- `kicad_schematic` reads **one sheet**. A hierarchical design lists its child sheets
  under `sheets`; read each by its `file` (relative to the root sheet's folder).
- `kicad_netlist` on the **root** sheet gives every net with its pins as `REF.PIN`. It is
  KiCad's own netlist, so it resolves labels, power symbols and hierarchy for you.
- `kicad_board` gives footprints with side and position, copper layers, counts, and the
  board outline's size in mm.
- `kicad_render` returns an image of the board. Look at it before describing a board, and
  after any change made in the GUI.

## Checking

- `kicad_erc` on the root sheet, `kicad_drc` on the board. Both return counts by
  severity and type first: read those, then the listed violations with their positions
  in mm.
- `kicad_drc` with `schematic_parity: true` also lists where the board no longer matches
  the schematic, which is what you need after schematic edits.
- Violations the designer excluded in KiCad are not counted.

## Changing part fields

`kicad_set_fields {file, reference, fields}` sets Value, Footprint, Datasheet, MPN,
Manufacturer or any other field of one part. The call:

- changes every unit of a multi-unit part, and adds a missing field hidden;
- changes only those values in the file, and keeps the old file under `.archie/history/`
  (the answer names it, so you can tell the person how to undo);
- refuses `Reference`, which KiCad's annotation owns;
- refuses while KiCad has the sheet open, because saving from KiCad would undo the edit.
  Ask the person to close the schematic editor, or make the change in the GUI.

A footprint set in the schematic reaches the board only through KiCad's **Update PCB
from Schematic** (F8 in the board editor). That step is done in the GUI. Afterwards run
`kicad_drc` with `schematic_parity`.

## Exports

`kicad_export {board?, schematic?, kinds}` writes into a new `exports/<design>-<n>/`
folder, with one subfolder per kind, and never replaces anything.

| Kind | Needs | What you get |
|---|---|---|
| gerbers | board | every copper layer, paste, silkscreen, mask, outline |
| drill | board | Excellon drill files in mm, with a PDF map |
| positions | board | pick-and-place CSV in mm, both sides |
| step | board | 3D model |
| bom | schematic | BOM CSV: Reference, Value, Footprint, quantity, DNP |
| netlist | schematic | KiCad netlist |
| schematic_pdf | schematic | the sheets as PDF |
| schematic_svg | schematic | one SVG per sheet |

For a fab order, export `gerbers`, `drill`, `positions` and `bom` in one call. Run
`kicad_drc` first; do not send boards with errors.

## Traps

- Paths are relative to the workspace. `../` and absolute paths are refused.
- A sheet that exists but is not the root gives an incomplete ERC and netlist. Use the
  root sheet from `kicad_projects`.
- `kicad_render` needs KiCad 9 or later, and it takes a few seconds on a large board.
- Library tables live in the person's KiCad configuration. A part whose library is
  missing on this machine shows up in ERC as a library mismatch, not as a broken file.

## When to use the GUI instead

These tools read, check, set fields and export. Use KiCad's window on the Archie Stage
(app ids `kicad`, `eeschema`, `pcbnew`) to:

- draw new circuitry or add symbols;
- place footprints and route tracks;
- run Update PCB from Schematic;
- answer any dialog.

After GUI work, confirm it with `kicad_erc`, `kicad_drc` and `kicad_render` rather than
by looking.
