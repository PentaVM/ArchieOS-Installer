---
name: bridge-video
description: Make videos through Archie's video bridge — list and probe media, then render a timeline of cut clips, titles, music and fades with MLT (Kdenlive's engine). Use when a task asks for a video, a cut, a montage or a trailer from files the person has.
---

# Video through Archie

The bridge works in one media folder. Sources are files in it; renders go to its
`renders/` folder.

## The order that works

1. `video_sources` lists what you can use.
2. `video_probe` each source you plan to cut. Clips are cut in seconds, so you need each
   source's duration. Check frame size and whether it has audio.
3. `video_render {output, timeline}`:
   - `output` is one file name ending in `.mp4`. An existing name is refused, so pick a
     new one.
   - `timeline.clips` play end to end. Each has `source`, `out_s` and optionally `in_s`
     (default 0); keep `out_s` within the duration `video_probe` gave.
   - `titles` sit over the clips from `start_s` for `duration_s`, and must end before the
     video does.
   - `music` plays under everything at `volume` 0–1 (default 0.3).
   - `fade_s` fades from and to black.
   - `width`, `height` and `fps` default to 1920, 1080 and 30. Set them to the sources'
     when those differ.

The answer names the video and an `.mlt` project beside it.

## Traps

- Paths are relative to the media folder. `../` is refused.
- A render can take minutes, and it is stopped after 30.
- Titles are plain text. Styling and effects are not in the timeline.

## When to use the GUI instead

Open the `.mlt` project in Kdenlive on the Archie Stage (app id `org.kde.kdenlive`) for:

- effects;
- transitions other than the fade;
- colour grading;
- keyframed motion.

Render from Kdenlive when done.
