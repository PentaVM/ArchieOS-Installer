---
name: bridge-audio
description: Clean up and prepare audio through Archie's audio bridge — measure loudness, remove noise, trim silence, fade, level to a LUFS target, mix voice and music, convert formats, draw waveforms. Use for podcasts, voice-overs, music beds and any recording.
---

# Audio through Archie

## First, measure

`audio_files` lists the recordings in the workspace, and videos whose sound can be used.

`audio_inspect {file}` gives:

- duration, sample rate and channels;
- loudness:
  - integrated LUFS;
  - loudness range;
  - true peak (dBTP). Above −1 dBTP clips on some players.

## Cleaning a voice recording

`audio_process {file, steps, format}` runs the steps in order and writes a new file in `edits/`. This order works for most speech:

```json
{"file": "raw/interview.wav", "steps": [
  {"highpass": {"hz": 80}},
  "denoise",
  "trim_silence",
  {"compress": {"ratio": 3, "threshold_db": -20}},
  {"normalize": {"lufs": -16}},
  {"fade_in": {"seconds": 0.3}}, {"fade_out": {"seconds": 1}}
], "format": "wav"}
```

Loudness targets:

| Target | LUFS |
|---|---|
| Podcasts | −16 |
| Streaming music | −14 |
| Broadcast (EBU R128) | −23 |

Normalize last, after anything that changes level.

Every result has a waveform picture; `audio_waveform {file}` draws one for any recording. Look at it: a flat top means clipping, and a thin
line means it is too quiet.

## Mixing

`audio_mix {name, tracks: [{file, start_s, gain_db}], normalize}` places each track on
the timeline. Music under a voice usually sits 15–20 dB lower (`gain_db: -18`). The mix is
levelled to −16 LUFS unless `normalize` is false.

## Formats for other software

| Format | For |
|---|---|
| wav, aiff | 24-bit, Audition, Pro Tools, Logic, DaVinci |
| flac | lossless archive |
| mp3, m4a, ogg, opus | publishing |

`audio_convert {file, to}` writes a copy in another format to `exports/`. Deliver the WAV for further editing, and the MP3 or M4A for listening.

## Traps

- `trim` works in seconds of the file as it is at that step, so put it first.
- `denoise` at a high setting makes voices sound watery; the default of 12 dB is gentle.
- `speed` changes tempo without changing pitch.

## When to use the GUI instead

Use Audacity on the Archie Stage for:

- repairs at one particular moment;
- spectral editing;
- projects that already live in Audacity.
