---
name: bridge-blender
description: Render, assemble and convert 3D through Archie's blender bridge — product shots and turntables of a model, scene inspection, FBX/glTF/USD/OBJ exports for Maya, Unreal, Unity or the web. Use when a task needs a 3D picture or video, or a model in another 3D format.
---

# 3D through Archie

Blender runs headless. It opens `.blend` scenes and models: FBX, glTF/GLB, OBJ, STL, PLY,
USD and Alembic. It does not open CAD files: convert STEP with `freecad_export` to `stl`,
`obj` or `glb` first.

## Looking at a scene

`blender_scenes` lists the scenes and models in the workspace.

`blender_inspect {file}` gives:

- every object's type, position, rotation, size and materials;
- the frame range, fps, render engine, resolution, camera and lights;
- the models' centre and radius, useful for framing.

## Pictures

`blender_render {file, engine, samples, width, height, frame, studio}` writes to
`renders/` and returns the image.

- **Engines:**
  - `cycles` (the default) path-traces on the CPU with denoising. Use 64 samples for
    previews and 256 for finals.
  - `eevee` is fast, but needs a GPU.
  - `workbench` is fastest and flat, good for checking composition.
- **Studio:** a bare model gets a camera framed on it, three soft lights, a clay-blue
  material and a light backdrop. The answer's `studio_added` says what was added. Pass
  `studio: false` to render a lit `.blend` exactly as it is.

## Videos

`blender_animation {file, turntable, frames, fps, engine, deliver_to_project}` starts a
job:

- A **turntable** (the default) orbits the camera once around the model. 96 frames at 24
  fps is 4 seconds.
- `turntable: false` renders the scene's own animation.
- Frames render a dozen at a time, then become `animations/<name>-<n>/<name>.mp4` with a
  poster frame.
- Follow it with `archie_job_status`.

Cycles at 1600×1000 takes seconds to minutes per frame on a CPU. Size the job to the
time you have: fewer frames, fewer samples, or `workbench`.

## Assembling and exporting

- `blender_assemble {files, name}` imports several models into one new `.blend`.
- `blender_export {file, formats}`:

| Format | For |
|---|---|
| fbx | Maya, 3ds Max, Unity, Unreal. Textures are embedded. |
| glb, gltf | the web, three.js, most viewers |
| usdc, usdz | NVIDIA Omniverse, Apple AR, Houdini |
| obj, stl, ply | printing, older tools |
| abc | animation caches |

## When to use the GUI instead

Use Blender on the Archie Stage for:

- modelling and sculpting;
- UVs and materials by eye;
- hand animation.

Check the result with `blender_render`.
