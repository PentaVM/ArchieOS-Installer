---
name: bridge-obs
description: Run a show in OBS Studio through Archie's obs bridge — switch scenes, add and update text and picture overlays (lower thirds, scores, captions), record, and look at what is on air. Use when a task involves OBS, a stream's overlays or a screen recording.
---

# OBS through Archie

OBS must be open, with **Tools › WebSocket Server Settings › Enable** turned on. The bridge
talks to it on this machine only. If OBS asks for a password, the operator gives the
server `--program obs-password=<file holding it>`.

## Seeing the show

- `obs_status` gives the current scene, whether OBS is recording or streaming, fps, CPU
  and dropped frames.
- `obs_scenes` gives every scene and its sources, and which are visible.
- `obs_screenshot` returns a picture of what is on air, or of one `source`. Look at it
  after every change you make.

## Overlays

- `obs_add_overlay {scene, name, text, x, y, size, color}` adds a text overlay: bold,
  outlined, with a drop shadow.
- `obs_add_overlay {scene, name, file, x, y}` adds a picture from the workspace. A PNG
  with transparency works best.
- `obs_set_text {input, text}` updates a text overlay live: a score, a caption, the next
  speaker's name.
- `obs_set_image {input, file}` swaps an overlay's picture.
- `obs_show {scene, source, visible}` shows or hides a source.

A lower third on a 1920×1080 canvas sits at about `x: 80, y: 900`, size 48.

Pictures for overlays can come from other bridges, for example a title card made with
`inkscape_export` as PNG.

## Scenes and recording

- `obs_switch_scene {scene}` puts a scene on air.
- `obs_record {action}`:
  - `start` records into the workspace's `recordings/`;
  - `stop` returns the file, ready for the video bridge or `archie_deliver`;
  - `pause` and `resume`.

## What this bridge never does

It does not start or stop a live stream, and does not touch stream keys or accounts.
Going live publishes in the person's name, so they do that in OBS's own window.

## Traps

- Overlay and scene names must match exactly. Read them with `obs_scenes`.
- Adding an overlay whose name is taken fails. Pick a new name, or use `obs_set_text` on
  the existing one.
