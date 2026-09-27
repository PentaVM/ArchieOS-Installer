# Delivery: results on the person's own computer

Archie often works somewhere else: a server, a rented GPU box, a VM on the workstation
itself. It may spend two hundred hours sweeping a FreeCAD design, rendering a turntable
in Blender or building a board in KiCad. Delivery brings every stage of that work to a
folder on the person's computer. There, their own software opens it:

- a STEP in Fusion 360, SolidWorks or COMSOL;
- a PSD in Photoshop, a DOCX in Word;
- an ODB++ in the board house's viewer;
- an FBX in Maya, an EDL in Premiere.

It is part of the OS, not of one bridge. Every bridge's files can be delivered with
`archie_deliver`, and long jobs (`freecad_sweep`, `blender_animation`) deliver on their
own as they go.

```text
node (server, VM, cloud)                                    workstation
  bridge exports ─► archie_deliver ─► 127.0.0.1:7447 ═══ ssh -R ═══► archie-desk ─► ~/Archie/<project>/
  long jobs ──────────────┘   queued in .archie/outbox while           stages/<stage>/…  every stage, kept
                              the desk is away, sent first              latest/…          newest of each file
                              when it is back                           index.html        gallery, reloads itself
```

## Setting it up

**1. On the workstation, pair and run the desk.**

`archie-desk` is one small program for Windows, macOS or Linux.

```bash
archie-desk pair
```

This writes `~/.archie/desk.key` and prints what to do on the node. Then:

```bash
archie-desk serve
```

It listens on `127.0.0.1:7447` only. Deliveries land in `~/Archie/`, and
`~/Archie/index.html` shows every project.

**2. On the node, give the MCP server the same key.**

```bash
install -m 600 /dev/null /etc/archie/deliver.key
```

Paste the 64 characters from the workstation's `~/.archie/desk.key` into that file, then
start the server with the key:

```bash
archie-mcp-server --clearance internal --deliver-key /etc/archie/deliver.key --bridge freecad=/srv/work/cad
```

**3. From the workstation, open the tunnel while Archie works.**

```bash
ssh -N -R 7447:127.0.0.1:7447 you@node
```

The same works when the node is a VM on the workstation: the VM is the `node`.

Nothing listens on a network at either end: the node talks to its own loopback, and ssh
carries the bytes. When the tunnel is down, deliveries wait in the bridge's
`.archie/outbox/`. The next delivery, or `archie_delivery_flush`, sends them in order.

## Opening results in the person's software

`~/.archie/desk.toml` says which kinds of file the desk may open, and with what. Kinds
not listed are delivered and left for the person to open.

```toml
[open]
# The system's own association: .step opens in whatever the person set, e.g. Fusion 360.
step = "default"
f3d = "default"
pdf = "default"
# Or a program, with the file as its argument.
kicad_pcb = { program = 'C:\Program Files\KiCad\10.0\bin\pcbnew.exe' }
blend = { program = 'C:\Program Files\Blender Foundation\Blender 4.5\blender.exe', args = ["{file}"] }
```

A delivery asks for a file to be opened with `open: true`. The desk opens it only if its
kind is listed, and it runs the program directly, never through a shell.

## What each bridge delivers for which software

| Bridge | Deliver | Opens in |
|---|---|---|
| freecad | step, iges, stl, 3mf, pictures | Fusion 360, SolidWorks, Onshape, Inventor, COMSOL, ANSYS, slicers |
| kicad | gerbers + drill + positions + bom, odb, ipc2581, step, renders | fabs, Altium, Cadence, Fusion 360 |
| blender | fbx, glb, usdc/usdz, mp4 turntables | Maya, 3ds Max, Unreal, Unity, Omniverse, any player |
| office | docx, xlsx, pptx, pdf | Word, Excel, PowerPoint, Acrobat |
| inkscape | pdf, eps, emf, dxf, png | Illustrator, CorelDRAW, Office, laser cutters |
| gimp | psd, tif, png | Photoshop, Affinity, print |
| qgis | shp, gpkg, kml, tif | ArcGIS, Google Earth |
| video | mp4, mlt, edl | any player, Kdenlive, Premiere, DaVinci Resolve |
| audio | wav, flac, mp3 | Audition, Pro Tools, Logic |

## Security

The delivery key is shared by pairing and never leaves the two key files. The desk
accepts a delivery only when all of these hold:

- its HMAC-SHA256 verifies with that key, and covers project, stage, file name, SHA-256,
  size, time, note and the open request;
- its timestamp is within five minutes of the desk's clock;
- it has not been received before;
- the bytes that arrived hash to the signed SHA-256.

Names are plain: letters, digits, `.`, `-` and `_`. So a delivery cannot climb out of
`~/Archie/`, and a stage is never overwritten. A different file of the same name gets
`-2`, `-3`.
