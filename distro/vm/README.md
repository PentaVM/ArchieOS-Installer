# ArchieOS for VirtualBox

`ArchieOS.ova` is the system the ISO installs, already installed on a virtual disk: Core
(`archied`), its PostgreSQL database, the observatory (`archie-display` on DRM/KMS, no
compositor), `archie-displayd`, `archie-worldd`, cortex and sentry, started by the same
systemd units as on a physical machine. The Windows installer (`distro/windows`) imports
it; the launcher starts and stops it.

## The GPU

VirtualBox does not give a machine the NVIDIA GPU on Windows, and there is no CUDA driver
for a VirtualBox guest to write: NVIDIA's is closed. So the local model runs where the GPU
is. The launcher starts PrismML's Windows build of llama.cpp (the revision ArchieOS runs in
WSL) on Windows' loopback before the machine, and the machine reaches it through
VirtualBox's NAT at 10.0.2.2 (`nat-localhostreachable`). Inside, `archie-host-model` reads
the model's name from the guest property `/ArchieOS/Model/Name`, finds its port in
`archie-local`'s catalog, puts Windows' server on the machine's own loopback with
`systemd-socket-proxyd` (Core takes a local model only there, and the words never leave the
PC), and writes Core's manifest for it before `archied` starts.

`archie-guest-status` tells VirtualBox which services run (`/ArchieOS/Status`), every 20 s,
for the launcher's service list.

## Building it

As root on Arch Linux (the WSL builder works), from a checkout with the release binaries
built:

    distro/vm/build-vm.sh /mnt/d/ArchieOS-build/vm/ArchieOS.vdi

It stages the ISO's profile (`distro/archiso/build.sh` with `ARCHIE_PROFILE_ONLY`, which
builds and tests everything the ISO does), pacstraps its packages onto a GPT disk image
(ESP and root), copies the staged root, finalizes it with the installer's own
`finalize-target.sh` (kernel, initramfs for any machine, systemd-boot by PARTUUID, root
locked), adds VirtualBox's guest service and the two units above, puts the observatory on
the first screen and makes an `owner` account for the recovery console whose password is
set at its first login. Firmware, microcode and Wi-Fi are left out. Then, on Windows:

    distro\vm\Export-ArchieOS-Ova.ps1 -Vdi D:\ArchieOS-build\vm\ArchieOS.vdi -Out D:\ArchieOS-build -BootTest

registers the machine (UEFI, VMSVGA with 3D, NAT, audio), boots it once headless to its
login prompt and a picture of its screen, and exports `ArchieOS.ova` with its SHA256SUMS
line.

## Not in it yet

The voice and IRIS run in the WSL image, which has the desktop session and audio they
need; the VirtualBox machine is the appliance without them.
