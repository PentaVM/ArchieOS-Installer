#!/usr/bin/env bash
# Builds ArchieOS's VirtualBox disk: the system the ISO installs, installed on a virtual
# disk without booting the live image. distro/archiso/build.sh stages the profile (its
# binaries, units, reviewed database packages); this partitions a disk image, pacstraps
# those packages onto it, copies the staged root, and finalizes it as the installer does
# (distro/installer/finalize-target.sh), then adds what a VirtualBox machine needs:
# VirtualBox's guest service, the local model from the Windows host (archie-host-model),
# the observatory on the first screen, and an owner account. Out: a VDI.
#
#   sudo distro/vm/build-vm.sh OUT.vdi          (run as root on Arch Linux; WSL works)
#
# Env: ARCHIE_VM_WORK (default /var/tmp/archieos-vm), ARCHIE_VM_SIZE (default 32G),
# CARGO_TARGET_DIR (reused by build.sh), ARCHIE_VM_SKIP_PROFILE=1 to reuse a staged profile.
set -euo pipefail
(( EUID == 0 )) || { echo 'build-vm: run as root' >&2; exit 77; }
out=${1:?build-vm: OUT.vdi}
repo_root=$(realpath "$(dirname "$0")/../..")
# shellcheck source=../../scripts/lib/copy-tree.sh
. "$repo_root/scripts/lib/copy-tree.sh"
work=${ARCHIE_VM_WORK:-/var/tmp/archieos-vm}
size=${ARCHIE_VM_SIZE:-32G}
profile=$work/profile
raw=$work/archieos.raw
mnt=$work/mnt
for command in pacstrap arch-chroot genfstab sgdisk losetup mkfs.fat mkfs.ext4 qemu-img blkid; do
  command -v "$command" >/dev/null || { echo "build-vm: missing $command" >&2; exit 69; }
done
install -d "$work"

if [[ ${ARCHIE_VM_SKIP_PROFILE:-0} != 1 ]]; then
  ARCHIE_PROFILE_ONLY=$profile bash "$repo_root/distro/archiso/build.sh"
fi
[[ -d $profile/airootfs && -f $profile/packages.x86_64 ]] || { echo "build-vm: no staged profile in $profile" >&2; exit 66; }

# The live medium's own packages, and the live installer's, stay out of an installed disk;
# so does what only real hardware needs (firmware, microcode, Wi-Fi): a virtual machine
# has none of it.
mapfile -t live_only < <(printf '%s\n' mkinitcpio-archiso syslinux squashfs-tools linux-firmware amd-ucode intel-ucode iwd
  grep -v '^\s*#' "$repo_root/distro/installer/packages.x86_64" | grep .)
mapfile -t packages < <(grep -v '^\s*#' "$profile/packages.x86_64" | grep . | grep -vxF -f <(printf '%s\n' "${live_only[@]}") | sort -u)

cleanup() {
  # pacman-key leaves a gpg-agent running on the new root's keyring, which keeps it busy:
  # the disk would be converted while still mounted.
  pkill -f -- "--homedir $mnt" 2>/dev/null || true
  sleep 1
  sync
  mountpoint -q "$mnt" && umount -R "$mnt" || true
  [[ -n ${loop:-} ]] && losetup -d "$loop" 2>/dev/null || true
}
trap cleanup EXIT

rm -f "$raw"
truncate -s "$size" "$raw"
sgdisk --zap-all "$raw" >/dev/null
sgdisk -n1:0:+512M -t1:ef00 -c1:ESP -n2:0:0 -t2:8304 -c2:ArchieOS "$raw" >/dev/null
loop=$(losetup --find --partscan --show "$raw")
for _ in $(seq 1 20); do [[ -b ${loop}p2 ]] && break; sleep 0.5; done
mkfs.fat -F32 -n ESP "${loop}p1" >/dev/null
mkfs.ext4 -q -L archieos "${loop}p2"
install -d "$mnt"
mount "${loop}p2" "$mnt"
install -d "$mnt/boot"
mount "${loop}p1" "$mnt/boot"

pacstrap -C "$profile/pacman.conf" -K "$mnt" "${packages[@]}" virtualbox-guest-utils-nox
# The staged root, with modes of the image's own (a checkout read through a Windows drive
# says 0777 for everything), then the modes the ISO profile sets for its own files.
copy_tree "$profile/airootfs" "$mnt"
while IFS= read -r line; do
  [[ $line =~ \[\"(/[^\"]+)\"\]=\"([0-9]+):([0-9]+):([0-7]+)\" ]] || continue
  [[ -e $mnt${BASH_REMATCH[1]} ]] || continue
  chown "${BASH_REMATCH[2]}:${BASH_REMATCH[3]}" "$mnt${BASH_REMATCH[1]}"
  chmod "${BASH_REMATCH[4]}" "$mnt${BASH_REMATCH[1]}"
done < "$profile/profiledef.sh"
# By UUID, read from the partitions themselves: genfstab asks udev, which a WSL builder
# does not run, and then writes the builder's own devices (its loop device, its swap).
cat > "$mnt/etc/fstab" <<FSTAB
UUID=$(blkid -s UUID -o value "${loop}p2")  /      ext4  rw,relatime  0 1
UUID=$(blkid -s UUID -o value "${loop}p1")  /boot  vfat  rw,relatime,fmask=0077,dmask=0077  0 2
FSTAB
grep -q '^UUID=[0-9a-fA-F-]\{9,\} *\/ ' "$mnt/etc/fstab" || { echo 'build-vm: no UUID for the root partition' >&2; exit 65; }

# What only the live medium does: log in as root on tty1 and offer the installer.
rm -rf "$mnt/etc/systemd/system/getty@tty1.service.d"
rm -f "$mnt/etc/systemd/system/graphical.target.wants/archie-installer.service"
# Finalize as the installer does (kernel, initramfs preset, systemd-boot entry by
# PARTUUID, root locked). The initramfs is built for any machine, not this builder.
echo "${loop}p2" > "$mnt/etc/archie/install-target-device"
: > "$mnt/usr/share/archie/installer-packages"
arch-chroot "$mnt" /usr/lib/archie/finalize-install-target "${loop}p2"
sed -i "s/^PRESETS=.*/PRESETS=('default' 'fallback')\ndefault_options='-S autodetect'/" "$mnt/etc/mkinitcpio.d/linux.preset"
arch-chroot "$mnt" mkinitcpio -P
arch-chroot "$mnt" bootctl install --no-variables --esp-path=/boot

# VirtualBox: its guest service (guest properties, time), the model from Windows.
ln -sf /usr/lib/systemd/system/vboxservice.service "$mnt/etc/systemd/system/multi-user.target.wants/vboxservice.service"
install -Dm 0755 "$repo_root/distro/vm/archie-host-model" "$mnt/usr/lib/archie/host-model"
install -Dm 0644 "$repo_root/distro/vm/archie-host-model.service" "$mnt/etc/systemd/system/archie-host-model.service"
install -Dm 0644 "$repo_root/distro/vm/archie-host-model-proxy@.socket" "$mnt/etc/systemd/system/archie-host-model-proxy@.socket"
install -Dm 0644 "$repo_root/distro/vm/archie-host-model-proxy@.service" "$mnt/etc/systemd/system/archie-host-model-proxy@.service"
ln -sf ../archie-host-model.service "$mnt/etc/systemd/system/multi-user.target.wants/archie-host-model.service"
# What runs, told to VirtualBox for the Windows launcher's service list.
install -Dm 0755 "$repo_root/distro/vm/archie-guest-status" "$mnt/usr/lib/archie/guest-status"
install -Dm 0644 "$repo_root/distro/vm/archie-guest-status.service" "$mnt/etc/systemd/system/archie-guest-status.service"
install -Dm 0644 "$repo_root/distro/vm/archie-guest-status.timer" "$mnt/etc/systemd/system/archie-guest-status.timer"
install -d "$mnt/etc/systemd/system/timers.target.wants"
ln -sf ../archie-guest-status.timer "$mnt/etc/systemd/system/timers.target.wants/archie-guest-status.timer"
install -Dm 0755 "$repo_root/distro/wsl-operator/rootfs/usr/bin/archie-local" "$mnt/usr/bin/archie-local"
install -Dm 0644 "$repo_root/distro/wsl-operator/rootfs/usr/share/archie/local-models.tsv" "$mnt/usr/share/archie/local-models.tsv"
# The observatory runs on tty2: the machine's window shows it, not a login prompt. tty1
# keeps the recovery console (Ctrl+Alt+F1 in VirtualBox: Host+F1).
install -d "$mnt/etc/systemd/system/archie-display.service.d"
printf '[Service]\nExecStartPre=+/usr/bin/chvt 2\n' > "$mnt/etc/systemd/system/archie-display.service.d/10-vm-screen.conf"

# The person's account for the recovery console and sudo: no password until the first
# login, which asks for one (the console is the machine's own window).
arch-chroot "$mnt" useradd --create-home --groups wheel --shell /bin/bash owner
arch-chroot "$mnt" passwd --delete owner
arch-chroot "$mnt" chage --lastday 0 owner
install -Dm 0440 /dev/stdin "$mnt/etc/sudoers.d/10-wheel" <<< '%wheel ALL=(ALL:ALL) ALL'
arch-chroot "$mnt" visudo -c
ln -sf /usr/share/zoneinfo/UTC "$mnt/etc/localtime"
echo 'LANG=C.UTF-8' > "$mnt/etc/locale.conf"
: > "$mnt/etc/machine-id"
rm -rf "$mnt/var/cache/pacman/pkg"/*
refuse_world_writable "$mnt" || exit 65
du -sh "$mnt"

cleanup
trap - EXIT
# Converted only once nothing holds it: a disk copied while mounted boots on a dirty journal.
if mountpoint -q "$mnt" || losetup -j "$raw" | grep -q .; then
  echo "build-vm: $mnt is still in use; not converting" >&2
  fuser -vm "$mnt" >&2 || true
  exit 70
fi
install -d "$(dirname "$out")"
rm -f "$out"
qemu-img convert -p -f raw -O vdi "$raw" "$out"
rm -f "$raw"
ls -la "$out"
echo "build-vm: $out"
