# Observatory controls

The native Observatory opens on Orb. Its scene switcher is ordered Orb, Forge, World.
`--scene orb|forge|world` still selects a specific opening view.

Archie's live voice exposes three visual tools: `observatory_scene`, `orb_cinematic`,
and `orb_overlay`. They send one JSON line to the native display on
`127.0.0.1:7668`. The display accepts only these values:

- Scene: `orb`, `forge`, `world`.
- Cinematic profile: `field`, `crown`, `atlas`, `dense`. Changing profile opens Orb.
- Overlay: `hud`, `shell`, `ambient`, `guides`, `links`, `pulses`, `bloom`, each `on` or `off`.

The listener changes only the local view. It cannot run commands, access Stage,
or write to displayd. If the Observatory is closed, Archie reports that the local
window did not respond.

Forge includes a CLI tile that starts `foot` when a graphical Wayland or X11
session and `foot` are present. The EGLFS image has no window manager, so Forge
shows its existing launch limitation there.
