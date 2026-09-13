#!/usr/bin/env bash
set -euo pipefail

repo_root=$(realpath "$(dirname "$0")/../..")
export ARCHIE_SOURCE_SHA=${ARCHIE_SOURCE_SHA:-$(git -C "$repo_root" rev-parse HEAD)}
out_dir=$(realpath -m "${1:-$repo_root/dist}")
work_dir=${ARCHIE_WSL_WORK_DIR:-/var/tmp/archieos-wsl-build}
rootfs="$work_dir/rootfs"
rootfs_mounted=0

cleanup() {
  if (( rootfs_mounted )); then
    umount "$rootfs"
  fi
}
trap cleanup EXIT

if [[ $EUID -ne 0 ]]; then echo 'run as root inside Arch Linux' >&2; exit 1; fi
if [[ $(. /etc/os-release; echo "$ID") != arch ]]; then echo 'builder must be Arch Linux' >&2; exit 1; fi
case "$(realpath -m "$work_dir")" in /var/tmp/archieos-wsl-*) ;; *) echo 'unsafe work dir' >&2; exit 1;; esac
command -v pacstrap >/dev/null
command -v arch-chroot >/dev/null

rm -rf --one-file-system "$rootfs"
install -d "$rootfs" "$out_dir"
reviewed_repo="$work_dir/reviewed-repo"
rm -rf --one-file-system "$reviewed_repo"
bash "$repo_root/distro/database/stage-reviewed-packages.sh" "$reviewed_repo"
pacstrap -K -c "$rootfs" base ca-certificates curl git jq openssh postgresql ripgrep sudo zstd freecad python
# arch-chroot requires the target to be a mount point. Without this, pacman cannot
# determine free space and reports a false out-of-disk error for local packages.
mount --bind "$rootfs" "$rootfs"
rootfs_mounted=1
rm -f "$rootfs/etc/resolv.conf"
rm -f "$rootfs/boot"/vmlinuz-* "$rootfs/boot"/initramfs-*
cp -a "$repo_root/distro/wsl/rootfs/." "$rootfs/"
chmod 0755 "$rootfs/usr/lib/wsl/archie-oobe"

# Install only the reviewed, signed TimescaleDB and pgvector artifacts. They are
# copied into the chroot because WSL builds deliberately have no host mounts.
install -d "$rootfs/var/cache/archie-reviewed"
cp "$reviewed_repo"/*.pkg.tar.zst "$reviewed_repo"/*.pkg.tar.zst.sig \
  "$rootfs/var/cache/archie-reviewed/"
reviewed_packages=()
for package in "$reviewed_repo"/*.pkg.tar.zst; do
  reviewed_packages+=("/var/cache/archie-reviewed/$(basename "$package")")
done
[[ ${#reviewed_packages[@]} == 2 ]]
arch-chroot "$rootfs" pacman -U --noconfirm "${reviewed_packages[@]}"
rm -rf "$rootfs/var/cache/archie-reviewed"

arch-chroot "$rootfs" useradd --uid 1000 --create-home --shell /bin/bash archie
arch-chroot "$rootfs" passwd -l archie
printf 'archie ALL=(root) /usr/bin/systemctl start archie-workstation.target, /usr/bin/systemctl stop archie-workstation.target\n' > "$rootfs/etc/sudoers.d/archie-services"
chmod 0440 "$rootfs/etc/sudoers.d/archie-services"

export CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1
cargo build --locked --manifest-path "$repo_root/core/Cargo.toml" --release \
  -p archie-state -p archied -p archie-cli -p archie-workstation
install -m 0755 "$repo_root/core/target/release/archied" "$rootfs/usr/bin/archied"
install -m 0755 "$repo_root/core/target/release/archie" "$rootfs/usr/bin/archie"
install -m 0755 "$repo_root/core/target/release/archie-db" "$rootfs/usr/bin/archie-db"
install -m 0755 "$repo_root/core/target/release/archie-workstation-check" "$rootfs/usr/bin/archie-workstation-check"
install -d "$rootfs/usr/lib/archie"
install -m 0644 "$repo_root/distro/database/pacman.conf" "$rootfs/etc/pacman.conf"
install -Dm 0644 "$repo_root/distro/database/compatibility.json" \
  "$rootfs/usr/share/archie/database-compatibility.json"
install -Dm 0644 "$repo_root/distro/database/preflight.sql" \
  "$rootfs/usr/share/archie/database-preflight.sql"
for unit in archie-postgres-init archie-postgres archie-database; do
  install -m 0644 "$repo_root/distro/database/${unit}.service" \
    "$rootfs/usr/lib/systemd/system/${unit}.service"
done
for helper in initialize-cluster provision-node wait-postgres; do
  install -m 0755 "$repo_root/distro/database/${helper}.sh" \
    "$rootfs/usr/lib/archie/${helper}"
done
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
