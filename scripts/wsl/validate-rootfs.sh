#!/usr/bin/env bash
set -euo pipefail

rootfs=${1:?usage: validate-rootfs.sh ROOTFS}
rootfs=$(realpath "$rootfs")
test -f "$rootfs/etc/os-release"
grep -q '^ID=arch$' "$rootfs/etc/os-release"
grep -q '^systemd=true$' "$rootfs/etc/wsl.conf"
grep -q '^enabled=false$' "$rootfs/etc/wsl.conf"
grep -q '^defaultUid=1000$' "$rootfs/etc/wsl-distribution.conf"
grep -q '^defaultName=ArchieOS$' "$rootfs/etc/wsl-distribution.conf"
test ! -e "$rootfs/etc/resolv.conf"
test ! -e "$rootfs/boot/vmlinuz-linux"
test ! -e "$rootfs/boot/initramfs-linux.img"
test -x "$rootfs/usr/bin/archie"
test -x "$rootfs/usr/bin/archied"
test -x "$rootfs/usr/bin/archie-db"
test -x "$rootfs/usr/bin/archie-workstation-check"
test "$(stat -c %u "$rootfs/home/archie")" = 1000
if awk -F: '$2 !~ /^(!|\*)*$/ { exit 1 }' "$rootfs/etc/shadow"; then :; else
  echo 'password hash found in rootfs' >&2
  exit 1
fi
for unit in core cerebra vault router speech tool-runtime; do
  test -f "$rootfs/usr/lib/systemd/system/archie-$unit.service"
done
for unit in postgres-init postgres database; do
  test -f "$rootfs/usr/lib/systemd/system/archie-$unit.service"
done
grep -q '^Requires=archie-database.service$' \
  "$rootfs/usr/lib/systemd/system/archie-core.service"
grep -q '^ExecStart=/usr/bin/archied$' \
  "$rootfs/usr/lib/systemd/system/archie-core.service"
for package in postgresql timescaledb pgvector; do
  grep -q "^${package} " "$rootfs/package-versions.txt"
done
test -f "$rootfs/usr/share/archie/database-compatibility.json"
test -f "$rootfs/usr/share/archie/database-preflight.sql"
test -f "$rootfs/usr/share/archie/models/iris/champion/model-manifest.json"
test -f "$rootfs/usr/share/archie/models/iris/champion/model.int8.json"
for directory in / /etc /usr /usr/lib /usr/lib/systemd /usr/lib/systemd/system \
  /usr/lib/tmpfiles.d /usr/lib/wsl; do
  test "$(stat -c %a "$rootfs$directory")" = 755
done
if grep -rIlE --exclude-dir=share --exclude-dir=pacman \
    '(NEBIUS_API_KEY|OPENAI_API_KEY|ANTHROPIC_API_KEY)=[^[:space:]]+' "$rootfs"; then
  echo 'credential material found in rootfs' >&2
  exit 1
fi
echo 'rootfs validation passed'
