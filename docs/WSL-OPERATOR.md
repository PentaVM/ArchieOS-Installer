# ArchieOS Operator for WSL

An image you import into WSL to try Archie on your own machine: talk with it, have it
work a screen, cut video with it, and look at the desktop design.

| Command | What it is |
|---|---|
| `archie-talk` | Speech to speech. Say "Archie", ask, and it answers aloud and opens applications. |
| `archie-operate "…" --app …` | A reasoning model plans; IRIS watches the screen and works the mouse and keyboard. |
| `archie-video` | The video bridge: timelines rendered by MLT, each with a project Kdenlive opens. |
| `archie-design` | The desktop design, as a preview. It draws invented data and is connected to nothing. |
| `archie-ask` | Ask the reasoning model in a terminal; private values are veiled before they leave. |

## What you need

- Windows 11 (or Windows 10 with WSLg), with WSL 2: `wsl --install`.
- An NVIDIA GPU with the current Windows driver, for IRIS. WSL shares it; 6 GB of video
  memory is enough. On cards before the RTX 30 series IRIS runs in float32.
- A microphone and speakers, for the voice. Headphones are better: with speakers, Archie
  stops listening while it speaks.
- About 16 GB of disk, and a Nebius Token Factory key.

## Install

Download the three files of the release into one folder:

- `ArchieOS-Operator.wsl`
- `iris-beta21-step132000.pt`
- `install-archie-operator.ps1`

In PowerShell, in that folder:

```powershell
powershell -ExecutionPolicy Bypass -File .\install-archie-operator.ps1
```

It imports the image as the distribution `ArchieOS-Operator`, copies IRIS's weights in,
makes a folder named `ArchieOS` on your desktop, and runs the first-time setup: PyTorch
(about 3 GB), the voice (about 1 GB) and the Nebius key. The key is typed once, is not
shown, and is kept in the image at `~/.config/archie/nebius.key`, readable only by you.

## The shared folder

`ArchieOS` on your Windows desktop is `~/ArchieOS` inside the image. It is the one place
both sides see: put clips, pictures and documents there, and what Archie makes appears
there. The video bridge works in it and nowhere else.

## Talk

```bash
wsl -d ArchieOS-Operator
archie-talk                # or: archie-talk --headphones
```

A window opens: the Stage, where applications Archie opens appear. Say "Archie" and then
what you want, in Spanish or English: "Archie, abre Kdenlive", "Archie, what time is it".
Moving your mouse over the Stage takes control back from it.

Hearing (Nemotron 3.5 ASR) and speaking (Piper) run on your machine, on the CPU. Only
the words of what you ask go to the reasoning model on Nebius, so talk as you would in
public. It answers with the essential first, in a sentence or two, and says more when
the conversation calls for it or you ask.

### A reasoning model on the machine

The installer offers to set up Bonsai 4B (PrismML, Apache 2.0) on your GPU. Say yes and it
fetches the model (572 MB), installs the CUDA toolkit (about 8 GB) and builds the runtime,
a quarter of an hour in all. From then on `archie-talk` uses it, nothing said leaves the
machine and no Nebius credit is spent on talking; `archie-talk --cloud` goes back to
Nemotron on Nebius. Later, or again:

```bash
archie-local setup 4b                                  # in the image
wsl -d ArchieOS-Operator -u root archie-local gpu      # from Windows
archie-local status
```

**This is the least tested part.** The GPU runtime was never built or run: the machine
the image was made on has no NVIDIA GPU. What was run is the same model on a six-core
CPU, through the voice's own path, and there it took 40 to 60 seconds to answer and
handled tools badly: asked what two plus two is, it proposed typing text, and its tool
calls came out as words to be spoken. The GPU fixes the speed. It may not fix the tools,
so "Archie, abre Kdenlive" may not open it with this model; if it does not, use
`archie-talk --cloud`.

## Operate a screen

```bash
archie-operate "do a flip reset" --app car-soccer
archie-operate "open the File menu and look at what it offers" --app photopea --seconds 45
archie-operate "add the first clip of the bin to the timeline" --app kdenlive
archie-operate "scroll down the page" --url https://en.wikipedia.org/wiki/Mona_Lisa
```

The first run downloads IRIS's text encoder (about 1.5 GB) and waits for the application
to load; nothing is timed until then. The plan is printed, then IRIS acts for the seconds
given (60 unless `--seconds` says otherwise). Each run is kept in
`~/archie-runs/operate/<run>/`: `result.json`, the plan and every decision in
`trajectory.jsonl`, and a frame a second in `frames/`.

The reasoning model is told the task and which application is open. It is not shown the
screen. Only IRIS sees the screen, and IRIS runs on your machine.

## Video

```bash
archie-video                 # what is in ~/ArchieOS
archie-video probe clip.mp4  # a clip's size and length
archie-video guide           # how a timeline is described
archie-video open            # Kdenlive, in the Stage
archie-video serve           # the same tools over MCP, for a planner
```

A render goes to `ArchieOS/renders/`: the MP4, a `.mlt` project Kdenlive and Shotcut
open, and an `.edl` cut list Premiere, Resolve and Final Cut import. To render from a
shell, with a timeline as the guide describes it:

```bash
archie-bridges call video video_render '{"output":"corte.mp4","timeline":{"clips":[{"source":"clip.mp4","in_s":2,"out_s":8}],"fade_s":0.5}}' --workspace ~/ArchieOS
```

## The design

```bash
archie-design
```

Xavi's prototype of the desktop: top bar, dock, the Vault file manager, control centre,
activity centre, command palette. Keys `1` to `4` switch between the three directions and
the proposed synthesis. Every screen says its data is invented. It is here to be looked
at and judged, not used.

## Other commands

| Command | What it does |
|---|---|
| `archie-setup status` | what is in place: Python, weights, voice, key, folder, GPU |
| `archie-setup key` | replace the Nebius key |
| `archie-session status` / `down` | the Stage, IRIS and the application: running or not; stop them |

## What was checked, and what was not

Checked, in the image or with its own binaries:

- The video bridge listing, probing and rendering a clip, on a Linux folder and on a
  Windows folder, with no display in the environment.
- The planner (Nemotron, 3.4 s), the Stage opening a browser in WSLg, input reaching the
  page and the run being recorded, with a stand-in answering for IRIS.
- The first-time setup of Python 3.14 and PyTorch; that Piper installs under it.
- The design prototype loading under Qt's own runner.

**Not checked:**

- **IRIS itself.** The machine this was built on has no NVIDIA GPU and 7 GB of memory;
  loading IRIS on its CPU was killed for lack of memory. On a GPU this is its first run.
  If it does not start, `~/.archie/run/iris.log` says why.
- **Xavi's shell bridge** (`archie-shell-bridge`) is in the image; `archie-design` does
  not feed the preview from it yet, so the preview's link chip reads "sin lectura".
- **The voice, end to end.** `archie-voice` was built and starts; a conversation through
  a microphone and speakers in WSLg was not held from this image.
- **Kdenlive in the Stage**, by voice or with IRIS.
- **The installer** on a second machine, and the shared folder it makes.

## Limits

- IRIS Beta 2.1 is a research model, trained mostly on a browser car-football game and on
  pointer steps in desktop applications. On held-out GUI steps its pointer lands within
  5 % of the target 60 % of the time. It will often miss. In `archie-operate` it moves,
  clicks and presses keys; it types no text and draws no strokes.
- Applications draw with software rendering inside the Stage. A game is slow.
- `archie-talk` and `archie-operate` each start their own Stage; use one at a time.
- No Observatory. `archie-lan` and `archie-netwatch` are in the image, but a WSL machine
  sees only its own virtual network, and TShark is not installed.
- An image built with `ARCHIE_OPERATOR_APPS=` has no Kdenlive; then, from Windows, once:
  `wsl -d ArchieOS-Operator -u root pacman -Sy --noconfirm kdenlive`.
- IRIS's weights are under a non-commercial research licence (`/opt/archie/iris/LICENSE`).

## Remove

```powershell
wsl --unregister ArchieOS-Operator
```

The `ArchieOS` folder on the desktop is yours and stays.
