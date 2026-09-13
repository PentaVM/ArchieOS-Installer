#!/usr/bin/env bash
set -euo pipefail

rootfs=${1:?usage: validate-rootfs.sh ROOTFS}
rootfs=$(realpath "$rootfs")
test -f "$rootfs/etc/os-release"
grep -q '^ID=arch$' "$rootfs/etc/os-release"
grep -q '^systemd=true$' "$rootfs/etc/wsl.conf"
grep -q '^enabled=false$' "$rootfs/etc/wsl.conf"
grep -q '^defaultUid=1000$' "$rootfs/etc/wsl-distribution.conf"
test ! -e "$rootfs/etc/resolv.conf"
test ! -e "$rootfs/boot/vmlinuz-linux"
test ! -e "$rootfs/boot/initramfs-linux.img"
test -x "$rootfs/usr/bin/archie"
test -x "$rootfs/usr/bin/archied"
test -x "$rootfs/usr/bin/archie-workstation-check"
test "$(stat -c %u "$rootfs/home/archie")" = 1000
for unit in core cerebra vault router speech tool-runtime; do
  test -f "$rootfs/usr/lib/systemd/system/archie-$unit.service"
done
if grep -RIlE --exclude-dir=share --exclude-dir=pacman \
    '(NEBIUS_API_KEY|OPENAI_API_KEY|ANTHROPIC_API_KEY)=[^[:space:]]+' "$rootfs"; then
  echo 'credential material found in rootfs' >&2
  exit 1
fi
echo 'rootfs validation passed'
