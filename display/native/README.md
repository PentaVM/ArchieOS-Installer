# Observatory controls

The native Observatory opens on Orb. Its scene switcher is ordered Orb, Forge, World.
`--scene orb|forge|world` still selects a specific opening view.

Archie's live voice exposes two visual tools: `observatory_scene` and `orb_preset`. The same
controls are on the CLI: `archie visual scene <orb|forge|world>`, `archie visual preset
<field|sphere|orb|neural>` and `archie visual list`. They send one JSON line to the native display on
`127.0.0.1:7668`. The display accepts only these values:

- Scene: `orb`, `forge`, `world`.
- Preset: `field` (Regiones vivas, the original look), `sphere`, `orb`, `neural`. Applying a preset
  opens Orb. Every visual parameter belongs to a preset; there are no overlay toggles.

The listener changes only the local view. It cannot run commands, access Stage,
or write to displayd. If the Observatory is closed, Archie reports that the local
window did not respond.

Forge includes a CLI tile that starts `foot` when a graphical Wayland or X11
session and `foot` are present. The EGLFS image has no window manager, so Forge
shows its existing launch limitation there.
