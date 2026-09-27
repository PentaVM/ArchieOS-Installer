---
name: bridge-freecad
description: Design and iterate mechanical parts through Archie's freecad bridge — build parametric solids, make variants, sweep parameters for hours, check volume and validity, export STEP/IGES/STL/3MF for Fusion 360, SolidWorks, COMSOL or a printer. Use when a task involves a physical part, a CAD file or a design study.
---

# FreeCAD through Archie

The bridge works in one folder of designs. Every path is relative to it. FreeCAD runs
headless. Pictures come back as images you can look at.

## Before the first call

- `freecad_documents` lists what exists.
- `freecad_inspect` on a design gives, for each resulting solid:
  - volume in mm³, area, bounding box and size in mm;
  - whether it is a valid solid;
  - its named parameters, the spreadsheet aliases that drive the model.

  Read it before changing anything.

## Making a part

`freecad_build {name, parameters, parts}` writes `designs/<name>-<n>.FCStd` and a picture.

- **parameters** become a spreadsheet called `Params`, e.g. `{"width": 80, "wall": 3}`.
  Numbers are millimetres.
- **parts** are built in order, and later parts refer to earlier ones by name:
  - primitives:
    - `box` (length, width, height)
    - `cylinder` (radius, height)
    - `sphere` (radius)
    - `cone` (radius1, radius2, height)
    - `torus` (radius1, radius2)
    - `wedge`
  - booleans:
    - `cut` (base minus tools)
    - `fuse`
    - `common`
  - finishing: `fillet` or `chamfer` (`radius`, and `edges` either "all" or a list of edge numbers).
- A dimension or an `at` coordinate is a number, or an expression over the parameters
  such as `"wall*2"` or `"width/2 - 10"`. Expressions keep the part parametric, which is
  what later variants and sweeps change.
- `rotate {axis: [0,0,1], angle: 90}` turns a part about its own origin.

Example, a plate with a hole:

```json
{"name": "plate", "parameters": {"w": 80, "d": 40, "t": 5, "hole": 8},
 "parts": [
   {"name": "Plate", "kind": "box", "length": "w", "width": "d", "height": "t"},
   {"name": "Hole", "kind": "cylinder", "radius": "hole/2", "height": "t*3", "at": ["w/2", "d/2", "-t"]},
   {"name": "Bracket", "kind": "cut", "base": "Plate", "tools": ["Hole"]}]}
```

Check the answer. `valid` must be true, and `results` should hold the one solid you meant.
If a boolean left two results, a tool did not overlap its base.

## Variants and long studies

- `freecad_set_parameters {file, parameters}` saves a new `variants/<design>-<n>.FCStd`.
  The original is never changed.
- `freecad_sweep {file, parameters: {alias: [values…]}, formats, deliver_to_project}`
  starts a job that tries every combination:
  - Each variant gets its own folder under `sweeps/`, with the model, the exports and a
    picture.
  - `summary.csv` holds one row per variant: parameters, validity, volume and size.
  - The job keeps running after this conversation.
  - With `deliver_to_project`, each variant reaches the person's desk as it finishes, and
    the summary arrives last.
  - Follow it with `archie_job_status` and stop it with `archie_job_cancel`.

  Size a sweep deliberately: each variant takes seconds to minutes, depending on the model.

## Exports for other software

`freecad_export {file, formats}` writes `exports/<design>-<n>/` with a picture.

| Format | For |
|---|---|
| step | Fusion 360, SolidWorks, Onshape, Inventor, CATIA, COMSOL, ANSYS |
| iges | older CAD and CAM |
| brep | OpenCASCADE tools |
| stl, 3mf | printing and slicers |
| obj, ply | rendering, Blender |
| fcstd | FreeCAD itself |

To put the result on the person's computer, call `archie_deliver` with the files. Their
own Fusion 360 or COMSOL then opens the STEP.

## Looking at a design

`freecad_render {file, view}` draws it from `iso`, `front`, `back`, `top`, `bottom`, `left`
or `right`. Look before describing a shape, and after every change.

## Traps

- Expressions refer to parameters by their names in `Params`. A name the spreadsheet does
  not have makes the recompute fail. The error says which expression failed.
- A fillet radius too large for an edge fails the whole fillet. Try a smaller one, or list
  the edges.
- STEP and mesh files have no parameters. Inspect and export them, but make variants only
  from an `.FCStd` with aliases.
- Units are millimetres throughout.

## When to use the GUI instead

Use FreeCAD's window on the Archie Stage for:

- sketches of free-form profiles;
- picking particular faces or edges by eye;
- TechDraw drawings;
- assemblies.

Check that work with `freecad_inspect` and `freecad_render`.
