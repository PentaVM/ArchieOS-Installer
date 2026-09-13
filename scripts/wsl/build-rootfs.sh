#!/usr/bin/env bash
set -euo pipefail

repo_root=$(realpath "$(dirname "$0")/../..")
export ARCHIE_SOURCE_SHA=${ARCHIE_SOURCE_SHA:-$(git -C "$repo_root" rev-parse HEAD)}
out_dir=$(realpath -m "${1:-$repo_root/dist}")
work_dir=${ARCHIE_WSL_WORK_DIR:-/var/tmp/archieos-wsl-build}
rootfs="$work_dir/rootfs"

if [[ $EUID -ne 0 ]]; then echo 'run as root inside Arch Linux' >&2; exit 1; fi
if [[ $(. /etc/os-release; echo "$ID") != arch ]]; then echo 'builder must be Arch Linux' >&2; exit 1; fi
case "$(realpath -m "$work_dir")" in /var/tmp/archieos-wsl-*) ;; *) echo 'unsafe work dir' >&2; exit 1;; esac
command -v pacstrap >/dev/null
command -v arch-chroot >/dev/null

rm -rf --one-file-system "$rootfs"
install -d "$rootfs" "$out_dir"
pacstrap -K -c "$rootfs" base ca-certificates curl git jq openssh ripgrep sudo zstd freecad python
rm -f "$rootfs/etc/resolv.conf"
rm -f "$rootfs/boot"/vmlinuz-* "$rootfs/boot"/initramfs-*
cp -a "$repo_root/distro/wsl/rootfs/." "$rootfs/"
chmod 0755 "$rootfs/usr/lib/wsl/archie-oobe"

arch-chroot "$rootfs" useradd --uid 1000 --create-home --shell /bin/bash archie
arch-chroot "$rootfs" passwd -l archie
printf 'archie ALL=(root) /usr/bin/systemctl start archie-workstation.target, /usr/bin/systemctl stop archie-workstation.target\n' > "$rootfs/etc/sudoers.d/archie-services"
chmod 0440 "$rootfs/etc/sudoers.d/archie-services"

export CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1
cargo build --manifest-path "$repo_root/core/Cargo.toml" --release -p archied -p archie-cli -p archie-workstation
install -m 0755 "$repo_root/core/target/release/archied" "$rootfs/usr/bin/archied"
install -m 0755 "$repo_root/core/target/release/archie" "$rootfs/usr/bin/archie"
install -m 0755 "$repo_root/core/target/release/archie-workstation-check" "$rootfs/usr/bin/archie-workstation-check"
install -d "$rootfs/usr/lib/archie"
install -m 0755 "$repo_root/scripts/models/install-nemotron-speech.sh" "$rootfs/usr/lib/archie/install-nemotron-speech"
install -m 0755 "$repo_root/scripts/freecad/smoke-test.sh" "$rootfs/usr/lib/archie/freecad-smoke-test"
arch-chroot "$rootfs" systemctl enable archie-workstation.target
arch-chroot "$rootfs" systemctl mask systemd-resolved.service systemd-networkd.service \
  NetworkManager.service systemd-tmpfiles-clean.service systemd-tmpfiles-clean.timer \
  systemd-tmpfiles-setup-dev-early.service systemd-tmpfiles-setup-dev.service tmp.mount
arch-chroot "$rootfs" pacman -Q > "$rootfs/package-versions.txt"
rm -rf "$rootfs/var/cache/pacman/pkg"/* "$rootfs/var/log"/*
rm -f "$rootfs/etc/machine-id"
: > "$rootfs/etc/machine-id"
"$repo_root/scripts/wsl/package-wsl.sh" "$rootfs" "$out_dir"
