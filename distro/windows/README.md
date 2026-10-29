# ArchieOS on Windows

## ArchieOS-Setup.exe

The one file the downloads page offers. It carries the launcher, its engine and texts, the
brand icon and the signed release manifest (`setup/payload.txt`). The first run installs the
launcher for this Windows user (`Install-ArchieOS-Launcher.ps1`, no administrator) and opens it
on Install; the exe stays as `%LOCALAPPDATA%\ArchieOS\Launcher\ArchieOS.exe`, which the shortcuts
open. A newer download installs itself over the old one; a missing file is put back the same
way. `--install` installs with no window (exit code 0 or 1); every install writes
`%TEMP%\ArchieOS-Setup.log`.

Source: `setup/ArchieOSSetup.cs` (C# 5, .NET Framework 4: the compiler every Windows has).
`setup/Build-ArchieOS-Setup.ps1` builds it on Windows with that compiler, `setup/build.sh` on
Linux with the .NET SDK's (deterministic: the same sources give the same exe), and
`setup/test-setup-exe.sh` checks its logic there against a stand-in for PowerShell. The window
and what Windows does (the script, the shortcuts, Settings → Apps) are checked on Windows only.
`scripts/release/New-Release.ps1` builds it with the manifest it has just signed.

## Installing

`Install-ArchieOS.cmd` (double-click; no console window). It reads the machine, proposes
how ArchieOS fits it, lets the person change any of it, and installs:

| | |
|---|---|
| Reads | Windows build and architecture, CPU, memory, each GPU's real memory (from the registry: WMI stops at 4 GB) and driver, drives, WSL, VirtualBox |
| Proposes | WSL or VirtualBox, the local model that fits (`local-models.tsv`), the drive (not Windows' own when another has room), memory and cores, the start at sign-in |
| WSL | installs WSL if missing (one UAC prompt; the installer reopens after the restart), sets `memory=` in `%USERPROFILE%\.wslconfig`, imports the image and runs `install-archie-operator.ps1 -Unattended -Model …` |
| VirtualBox | installs VirtualBox if missing (Oracle's SHA256SUMS), fetches PrismML's Windows runtime for the GPU found (CUDA, HIP, Vulkan, CPU) and the model, imports `ArchieOS.ova` with the chosen resources (see `distro/vm/README.md`) |
| Then | the launcher, with its settings in `%APPDATA%\ArchieOS\launcher.json` |

`Install-ArchieOS.ps1 -DryRun` shows the steps and changes nothing. `-Source DIR` points at
the release folder (the image, `ArchieOS.ova`, `SHA256SUMS`, `local-models.tsv`); by default,
the installer's own folder. An ARM PC is told the images are x64 only.

## The launcher

`ArchieOS.lnk` on the desktop and in the Start menu (`%LOCALAPPDATA%\ArchieOS\Launcher`).
It starts, restarts and stops the node, opens the GUI and the voice, fetches and switches
local models, and shows each service. At sign-in it waits a few seconds with "Iniciar
ahora", "No iniciar esta vez" and "No iniciar con Windows". It warns when WSL's virtual disk
holds far more than Linux uses and compacts it (`Compact-ArchieOS-Disk.ps1`, elevated).
With VirtualBox it starts the model on Windows first, hidden, and stops it with the machine.

`launcher.json`, which the installer writes and the person may edit:

| Key | |
|---|---|
| `backend` | `wsl` or `virtualbox` |
| `distro`, `vm` | the WSL distribution, the VirtualBox machine |
| `autostart_delay_s` | seconds of warning at sign-in (0: none). Whether it starts at sign-in at all is the Startup shortcut, switched in the launcher |
| `open_gui_on_start`, `voice_on_start` | open the GUI, the voice, when the node is up |
| `voice_always` | the voice weighs everything it hears, its name or not (default); off, only what starts with "Archie" |
| `host_model` | VirtualBox only: the model served from Windows (name, port, file, runtime) |

Settings → Apps lists ArchieOS; `Uninstall-ArchieOS-Launcher.ps1` removes the launcher, its
shortcuts and settings, and the WSL distribution only when named and confirmed.
