---
name: bridge-inkscape
description: Make and edit vector graphics through Archie's inkscape bridge — logos, diagrams, labels, laser-cut and print files — and export PDF/EPS/EMF/DXF/PNG for Illustrator, CorelDRAW, Office or a cutter. Use when a task needs a vector drawing.
---

# Vector drawings through Archie

## Making a drawing

Write SVG yourself and save it with `inkscape_write_svg {name, svg}`. The answer includes
a picture: look at it before going on. The bridge refuses scripts, event handlers, embedded
HTML and links to the network, so keep drawings self-contained.

Good SVG for later work:

- Give the shapes you will edit an `id`.
- Set `width`, `height` and a `viewBox` in real units, e.g. `width="100mm"`.
- Put styling in attributes such as `fill` and `stroke`.

## Reading

- `inkscape_drawings` lists the drawings.
- `inkscape_inspect` gives the size and each object's `id` and bounding box, as Inkscape
  measures them after transforms. Use those ids.

## Editing

`inkscape_edit {file, operations}` applies operations in order and writes
`edits/<name>-<n>.svg`. Operations:

- **Selecting:**
  - `{"select": ["id1", "id2"]}`
  - `{"select_class": "c"}`
  - `"select_all"`
  - `"deselect"`
- **Paths:**
  - `"to_path"`
  - `"stroke_to_path"`
  - `"union"`, `"difference"`, `"intersection"`, `"exclusion"`
  - `"combine"`, `"break_apart"`
  - `"simplify"`
- **Arranging:**
  - `"group"`, `"ungroup"`
  - `"duplicate"`, `"delete"`
  - `"raise_to_top"`, `"lower_to_bottom"`
- **Transforming:**
  - `{"rotate": 90}`
  - `{"scale": 1.5}`
  - `{"translate": [10, 0]}`
  - `{"flip": "horizontal"}`
- **Styling:**
  - `{"set_style": {"property": "fill", "value": "#2f6fde"}}`
  - `{"set_attribute": {"name": "opacity", "value": "0.5"}}`
- **Canvas:**
  - `{"fit_canvas": "drawing"}`
  - `"vacuum"`

For a laser cutter, turn text and strokes into outlines: select, `"to_path"`,
`"stroke_to_path"`, then export dxf or pdf.

## Exports

`inkscape_export {file, formats, area, dpi, id}` writes `exports/<name>-<n>/`.

| Format | For |
|---|---|
| pdf | Illustrator, print shops. Text becomes outlines, so fonts cannot go missing. |
| eps, ps | older print workflows |
| emf, wmf | Word, PowerPoint, CorelDRAW |
| dxf | laser cutters, CAD |
| png | the web and slides. Set `dpi`. |
| svg | plain SVG without Inkscape's extras, for browsers and other editors |

`area` is `page` (the default) or `drawing`, which is cropped to the art. `id` exports one
object alone.

## When to use the GUI instead

Use Inkscape's window on the Archie Stage for:

- freehand drawing;
- editing nodes by eye;
- anything easier to point at than to name.

Check the result with `inkscape_render`.
