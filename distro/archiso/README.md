# archiso

**Status: IMPL**

archiso profile. Consumed by `mkarchiso`. This is what builds the ISO.

Build on Arch Linux (including the `ArchieOS-Builder` WSL2 environment) with
`archiso`, `base-devel`, Rust, CMake, Ninja, QEMU, Qt6 base/declarative/quick3d/
multimedia and `ttf-dejavu` installed. Run `bash distro/archiso/build.sh OUTPUT_DIR`
as root. Keep source, work files and output separate; allow space for both the
uncompressed root filesystem and the compressed ISO. `CARGO_TARGET_DIR` and
`ARCHIE_ISO_WORK_DIR` can move build caches to another filesystem.

The UEFI boot menu offers the observatory and **Install ArchieOS - graphical
wizard**. Installation mode uses `archie.install=1`, enables only the live wizard
on tty2 and leaves tty1 for recovery. Without that flag, the normal observatory
starts. The native installer asks for the exact target disk before erasing it.

`distro/installer/packages.x86_64` contains the temporary compositor, terminal and
partition editor. They are included in the ISO, then removed from the installed
target. Finalization replaces the archiso initramfs preset, writes a PARTUUID boot
entry, removes live autologin, locks root and enables password-protected sudo for
the owner's wheel account. The bootloader is installed without changing firmware
boot order. No actual host disk should be used for installer testing.

Build output and `SHA256SUMS` alone do not certify boot. Run the resulting image
in QEMU, verify the control-plane readiness markers and test the UEFI installer
against a newly created virtual disk. The mobile companion in `tools/owner-companion`
is still a separate development service; it is not automatically provisioned in
this image. Camera/voice UI checks do not claim biometric identity recognition.
