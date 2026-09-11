#!/usr/bin/env bash
# Executed inside the reviewed target by arch-chroot, never by the live wizard.
set -euo pipefail
[[ $EUID == 0 && $# == 1 && -b $1 ]] || exit 64
[[ -f /etc/archie/install-target-device && $(cat /etc/archie/install-target-device) == "$1" ]] || exit 64
root_uuid=$(blkid -s PARTUUID -o value -- "$1")
[[ "$root_uuid" =~ ^[a-fA-F0-9-]+$ ]] || exit 65
# The installed machine boots its own disk, not the archiso squashfs on the USB.
rm -f -- /etc/mkinitcpio.conf.d/archiso.conf
cat >/etc/mkinitcpio.d/linux.preset <<'PRESET'
ALL_config='/etc/mkinitcpio.conf'
ALL_kver='/boot/vmlinuz-linux'
PRESETS=('default' 'fallback')
default_image='/boot/initramfs-linux.img'
fallback_image='/boot/initramfs-linux-fallback.img'
fallback_options='-S autodetect'
PRESET
cat >/etc/mkinitcpio.conf <<'INITRAMFS'
MODULES=()
BINARIES=()
FILES=()
HOOKS=(base udev autodetect microcode modconf kms keyboard keymap consolefont block filesystems fsck)
INITRAMFS
install -d /boot/loader/entries
printf 'default archieos.conf\ntimeout 3\neditor no\n' >/boot/loader/loader.conf
printf 'title ArchieOS\nlinux /vmlinuz-linux\ninitrd /initramfs-linux.img\noptions root=PARTUUID=%s rw\n' "$root_uuid" >/boot/loader/entries/archieos.conf
# Lock the passwordless root account inherited from the live image.
usermod --lock root
visudo -c
systemctl disable archie-installer.service
# Keep the observatory native; only the live session needs a window compositor.
packages=()
while IFS= read -r package; do
  [[ -z "$package" || "$package" == \#* ]] && continue
  [[ "$package" =~ ^[a-z0-9][a-z0-9+-]*$ ]] || exit 65
  pacman -Qq "$package" >/dev/null 2>&1 && packages+=("$package")
done </usr/share/archie/installer-packages
if ((${#packages[@]})); then pacman -R --noconfirm "${packages[@]}"; fi
rm -f -- /etc/archie/install-target-device
