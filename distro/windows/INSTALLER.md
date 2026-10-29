# Installing ArchieOS from the launcher

`ArchieOS-Launcher.ps1` (Windows PowerShell 5.1 + WPF, nothing to install) has an **Install ArchieOS**
section: it checks the PC, downloads the release image, verifies it, imports it into WSL 2, checks
every model (IRIS included), and then starts, stops, verifies and repairs it. The long steps run as
`ArchieOS-Installer.ps1` processes, so the window never freezes. The full audit and the state machine
are in [INSTALLER.es.md](INSTALLER.es.md) (Spanish).

| File | What it is |
|---|---|
| `ArchieOS-Launcher.ps1` | The window |
| `ArchieOS-Installer.ps1` | The engine (probe, plan, install, verify, repair, start, stop, configure) |
| `ArchieOS-i18n.ps1` | The texts: English by default; Spanish, Portuguese, French, German |
| `archieos-manifest.json` + `.sig` | The pinned release, and its signature |
| `Install-ArchieOS-Launcher.ps1` | Puts all of the above in `%LOCALAPPDATA%\ArchieOS\Launcher` with shortcuts |

## Signed releases

The manifest names what is downloaded (URL at a fixed revision, size, SHA-256). It is signed with
ECDSA P-256 / SHA-256 (`scripts/release/Sign-Release.ps1`); the installer carries the public key and
**refuses a manifest that is unsigned, altered, or signed by an unknown key**. The private key stays on
the release machine (sealed with DPAPI). `-AllowUnsigned` exists for development only and prints a warning.
The signature covers the manifest's exact bytes, so `.gitattributes` keeps them out of line-ending conversion.

## Languages

English is the default. **Advanced → Startup → Language** switches to Spanish, Portuguese, French or
German (the launcher reopens in it). Every text goes through `T 'English text'`; a text with no
translation shows in English. To add a language, add a table to `ArchieOS-i18n.ps1` and its name to
`$script:Languages`. `ARCHIE_LANG=es` forces a language for one run.

## Advanced settings

Install tab → **Archie's settings** (also saved to `%APPDATA%\ArchieOS\launcher.json`):

- **Listening**: only when called by name (default) or everything it hears.
- **Speech recognition language**: Spanish (es-US / es-CL), English (en-US) or automatic.
- **Voice**, **headphones** (keeps listening while it speaks) and **Wanda** on/off.

The installer writes them inside the distribution as `~/.config/archie/profile.json`
(`ArchieOS-Installer.ps1 configure -Root DIR`), and `archie-offline` reads them each time it starts
(`distro/wsl-operator/rootfs/usr/lib/archie/offline-session.py`, tested in `scripts/wsl/test_offline_bundle.py`).
Wanda starts only if `~/.config/archie/wanda/wanda.toml` exists: the image ships none. The local model is
chosen in the **Local model** tab (`archie-local`).

**These settings reach a running image only after it is rebuilt with this `offline-session.py`.** An image
built before it ignores the profile.

## Not done

Nothing here has downloaded or imported the real image: the signature, the settings profile (against a
mocked guest) and the Python profile logic are tested; the download, `wsl --import`, the first start and
the voice with a chosen language are not. VirtualBox and VMware install nothing yet (no OVA is published).
