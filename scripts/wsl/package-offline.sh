#!/usr/bin/env bash
# Package a clean staged root, never a running/personal WSL export.
set -euo pipefail
rootfs=$(realpath "${1:?usage: package-offline.sh CLEAN_ROOTFS OUTPUT.wsl}")
output=$(realpath -m "${2:?usage: package-offline.sh CLEAN_ROOTFS OUTPUT.wsl}")
[[ "$rootfs" != / && -f "$rootfs/usr/share/archie/offline/payload.json" ]]
[[ "$output" == *.wsl && ! -e "$output" && ! -e "$output.partial" ]]
case "$output" in "$rootfs"/*) echo 'output must be outside the image' >&2; exit 64;; esac
repo_root=$(realpath "$(dirname "$0")/../..")
python3 "$repo_root/scripts/wsl/image-keyring.py" check "$rootfs"
python3 "$repo_root/scripts/wsl/validate-offline-overlay.py" "$repo_root/distro/wsl-operator/rootfs" "$rootfs"
[[ ! -e "$rootfs/home/archie/.local/state" && ! -e "$rootfs/home/archie/.config" ]]
[[ ! -e "$rootfs/home/archie/.cache/huggingface/token" ]]
[[ ! -s "$rootfs/etc/machine-id" ]]
if find "$rootfs/etc/pacman.d/gnupg/private-keys-v1.d" "$rootfs/etc/ssh" \
    -type f \( -name '*.key' -o -name 'ssh_host_*_key' \) -print -quit | grep -q .; then
  echo 'private builder key found; refusing to package it' >&2; exit 65
fi
grep -q '^command=/usr/lib/wsl/archie-offline-oobe$' "$rootfs/etc/wsl-distribution.conf"
grep -q '^defaultName=ArchieOS-Bonsai-Xavi$' "$rootfs/etc/wsl-distribution.conf"
for file in usr/bin/archie-offline usr/lib/wsl/archie-offline-oobe usr/lib/archie/init-pacman-keyring usr/bin/archie-display usr/bin/archied; do
  test -x "$rootfs/$file"
done
python3 "$rootfs/usr/lib/archie/offline-session.py" --check --prefix "$rootfs"
python3 -c 'import json,sys; m=json.load(open(sys.argv[1])); s=m["source_sha"]; assert len(s)==40 and all(c in "0123456789abcdef" for c in s), "source commit is not finalized"' \
  "$rootfs/usr/share/archie/offline/payload.json"
mkdir -p "$(dirname "$output")"
partial="$output.partial"
( set -o noclobber; : > "$partial" )
trap 'rm -f -- "$partial"' EXIT
# One compression worker, low CPU priority: do not compete with active training.
tar --numeric-owner --xattrs --acls -C "$rootfs" -cf - . | nice -n 19 gzip -n -1 > "$partial"
gzip -t "$partial"
mv -- "$partial" "$output"
sha256sum "$output"
