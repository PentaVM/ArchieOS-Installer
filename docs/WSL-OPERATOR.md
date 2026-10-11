# ArchieOS Operator for WSL

An image you import into WSL to have Archie operate a screen: you say what to do, a
reasoning model plans it, and IRIS watches the screen and works the mouse and keyboard.

## What you need

- Windows 11 (or Windows 10 with WSLg), with WSL 2: `wsl --install`.
- An NVIDIA GPU with the current Windows driver. WSL shares it; nothing is installed in
  the image for it. 6 GB of video memory is enough. On cards before the RTX 30 series
  IRIS runs in float32.
- About 12 GB of disk, and a Nebius Token Factory key for the reasoning model.

## Install

Download the three files of the release into one folder:

- `ArchieOS-Operator.wsl`
- `iris-beta21-step132000.pt`
- `install-archie-operator.ps1`

In PowerShell, in that folder:

```powershell
powershell -ExecutionPolicy Bypass -File .\install-archie-operator.ps1
```

It imports the image as the distribution `ArchieOS-Operator`, copies IRIS's weights into
it, installs PyTorch inside it (about 3 GB) and asks for the Nebius key. The key is typed
once, is not shown, and is kept in the image at `~/.config/archie/nebius.key`, readable
only by you.

## Use

```bash
wsl -d ArchieOS-Operator
archie-operate "do a flip reset" --app car-soccer
archie-operate "open the File menu and look at what it offers" --app photopea --seconds 45
archie-operate "scroll down the page" --url https://en.wikipedia.org/wiki/Mona_Lisa
```

A window opens on the Windows desktop: the Stage, with a browser in it. The first run
downloads IRIS's text encoder (about 1.5 GB) and waits for the page to load; nothing is
timed until then. The plan is printed, then IRIS acts for the seconds given (60 unless
`--seconds` says otherwise).

Each run is kept in `~/archie-runs/operate/<run>/`: `result.json`, the plan and every
decision in `trajectory.jsonl`, and a frame a second in `frames/`.

| Command | What it does |
|---|---|
| `archie-setup status` | what is in place: Python, weights, key, GPU |
| `archie-setup key` | replace the Nebius key |
| `archie-session status` / `down` | the Stage, IRIS and the browser: running or not; stop them |
| `archie-ask` | ask the reasoning model in a terminal; private values are veiled first |

## What to expect

IRIS Beta 2.1 is a research model. It was trained mostly on a browser car-football game
and on pointer steps in desktop applications, and it is not good yet: on held-out GUI
steps its pointer lands within 5 % of the target 60 % of the time. It will often miss.
It moves, clicks and presses keys; in this image it types no text and draws no strokes.
What it does is recorded, so a run that goes wrong is worth as much as one that goes right.

The reasoning model is told what the task is and which application is open. It is not
shown the screen. Only IRIS sees the screen, and IRIS runs on your machine.

## Limits

- **IRIS was not run from this image before it shipped.** It was assembled on a machine
  with no NVIDIA GPU and 7 GB of memory, where loading IRIS on the CPU was killed for
  lack of memory. The setup, the Stage and the planner were checked; IRIS on a GPU, a
  GTX 1660 included, was not. If it fails to start, `~/.archie/run/iris.log` says why.
- The browser draws with software rendering inside the Stage. A game is slower than in
  a browser on Windows.
- No voice. No Observatory. `archie-lan` and `archie-netwatch` are in the image, but a
  WSL machine sees only its own virtual network, not your home network, and TShark is
  not installed.
- IRIS's weights are under a non-commercial research licence (`/opt/archie/iris/LICENSE`).

## Remove

```powershell
wsl --unregister ArchieOS-Operator
```
