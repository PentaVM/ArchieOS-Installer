# TFOG Orb rendering

The observatory has three places: **World** (the globe), **Forge** (the applications)
and **Orb** (the system as a living graph). CORE, the dependency node view, is gone: its
nodes repeated what the orb already holds.

## Orb

The orb draws the TFOG graph the way the Lumenfield design (the internal name of the orb
project) reads a tree. Nothing on screen names the project.

- **Composition (Rust, `archie-orb::lumen`, served at `/api/orb/lumen`).** The system is
  the root; entities with children are modules; the leaves of one parent are folded into
  one group per category until opened. Three lenses: architecture (folded), folders (the
  first two levels opened) and entities (one orb per entity). Anchors come from the
  design's region layout, with the model's own spectral direction standing in for the
  design's content signature. Relations are the observed edges (runtime, dependency,
  action, focus, accessibility) folded onto the orbs that hold their ends, then semantic
  neighbours where nothing was observed. Pure and deterministic; tested.
- **Motion and geometry (C++, `archie::visual::lumen`).** Spring physics to the anchors and
  a push from the pointer; the membrane, swollen slightly where orbs gather and tinted by
  the nearest; satellites and ghost particles, which are density only and never entities;
  relation arcs; the orb meshes. Tested in `test_lumen`.
- **Drawing (Qt Quick 3D, `LumenField`, `LumenScene.qml`).** Instanced orbs whose shape is
  pushed out of round by seeded noise and partly faceted, lit so the hue survives, with a
  glowing core, an iridescent rim and a halo. Qt Quick 3D gives custom materials a flat
  grey `INSTANCE_COLOR`, so colour, activity and attention travel packed in the instance
  data channel.

### What lights up

A node is **engaged** when the system is using it: a Stage action on it (predicted,
active, verified, error), Cortex holding it in focus or in a working set, or the voice
speaking about it. Load alone is not use. Engaged nodes burn brighter, their halo swells,
the way from them to the centre and their relations are lit, and signals travel those
links. The line at the bottom of the screen names the state (listening, thinking,
speaking, at rest) and the engaged nodes.

### Cinematics

The camera moves because something happened. A new field projects out of the centre while
the camera pulls in from far away (2.6 s); when the system engages a node the camera leans
towards it (1.6 s) and settles back when it lets go (2.2 s). Between events the view only
drifts a few degrees over a minute. A person can still turn it by dragging, approach with
the wheel, select with a click and open a module with a double click.

### Configuration

There are no configuration panels. The design's settings (profiles, density, sizes,
links, ghosts, surface, motion, bloom, colours) live in `LumenField` with the design's
defaults; setting them by command, from Cortex, is the next step.

## Forge

A desktop of large icons: Archie's own applications (shipped as desktop entries inside the
observatory), everything installed under the XDG application directories, and faintly
the well-known tools that are missing. A small light marks what is running. Clicking one
opens it when the observatory runs in a desktop session; the image runs the observatory on
eglfs, where there is nowhere to open a window, so there opening goes through the Stage.

## Budget

`ARCHIE_ORB_POLL_MS` (default 500 ms, 200–2000) sets how often the composed scene is
fetched. Only the scene on screen does work: the orb fetches and steps while it is shown,
Forge reads `/proc` while it is shown. Bloom goes off with the rest of the render budget.
