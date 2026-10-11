#!/usr/bin/env bash
# Builds ArchieOS-Operator.wsl: the image a person imports into WSL to have Archie operate
# a screen. In it: the Stage, a browser to work in, IRIS's code, the reasoning models'
# manifests, and Sentry's network tools. Not in it: PyTorch and IRIS's weights (too large
# for one release asset; archie-setup and the installer bring them) and any key.
#
#   IRIS_SRC=/path/to/archie-iris VJEPA_SRC=/path/to/vjepa2 STAGE=/path/to/archie-stage \
#     BIN=/path/to/target/release \
#     scripts/wsl/build-operator.sh [OUT_DIR | -]
#
# With `-` the image is written to standard output and nothing else is: for a builder
# with no room to hold the image beside the tree it is made from.
# Run as root on Arch Linux with arch-install-scripts.
set -euo pipefail
repo_root=$(realpath "$(dirname "$0")/../..")
out=${1:-$repo_root/dist}
exec 3>&1 1>&2
iris_src=${IRIS_SRC:?IRIS_SRC is the archie-iris checkout}
# facebookresearch/vjepa2 at the revision IRIS pins (MIT): the code its encoder is built from.
vjepa_src=${VJEPA_SRC:?VJEPA_SRC is facebookresearch/vjepa2 at the revision IRIS pins}
vjepa_revision=204698b45b3712590f06245fbfba32d3be539812
stage=${STAGE:?STAGE is the built archie-stage}
bin=${BIN:?BIN is the cargo release directory}
work_dir=/var/tmp/archieos-wsl-operator
rootfs=$work_dir/rootfs

[[ $EUID -eq 0 ]] || { echo 'run as root on Arch Linux' >&2; exit 1; }
command -v pacstrap >/dev/null
rm -rf --one-file-system "${work_dir:?}"
install -d "$rootfs"

# Qt for the Stage, Mesa's software GL (WSLg shares no GPU buffers with a nested
# compositor), and a font so a page has letters. No TShark: a WSL machine sees only its
# own virtual network, so archie-netwatch has nothing of the house to listen to here.
pacstrap -K -c "$rootfs" base ca-certificates curl jq python firefox mesa \
  qt6-base qt6-declarative qt6-wayland ttf-dejavu

rm -f "$rootfs/etc/resolv.conf" "$rootfs/boot"/vmlinuz-* "$rootfs/boot"/initramfs-*
cp -a "$repo_root/distro/wsl-operator/rootfs/." "$rootfs/"
chmod 0755 "$rootfs/usr/bin/archie-setup" "$rootfs/usr/bin/archie-session" "$rootfs/usr/bin/archie-operate"

install -m 0755 "$stage" "$rootfs/usr/bin/archie-stage"
for binary in archie-arena-live archie-stage-cli archie-ask archie-lan archie-netwatch archie-print archie-sentry-probe; do
  install -m 0755 "$bin/$binary" "$rootfs/usr/bin/$binary"
done

# Which models the node has: the reasoning models reached through Nebius, and IRIS.
while IFS= read -r -d '' manifest; do
  family=$(basename "$(dirname "$manifest")")
  install -Dm 0644 "$manifest" "$rootfs/etc/archie/models/$family/$(basename "$manifest")"
done < <(find "$repo_root/models/nebius" "$repo_root/models/iris" -maxdepth 1 -name '*.toml' -type f -print0)
install -Dm 0644 "$repo_root/ARCHIE.md" "$rootfs/usr/share/doc/archie-os/ARCHIE.md"
install -Dm 0644 "$repo_root/docs/WSL-OPERATOR.md" "$rootfs/usr/share/doc/archie-os/WSL-OPERATOR.md"

# IRIS's code. It is Python, the one part of Archie that is; its licence travels with it.
install -d "$rootfs/opt/archie/iris"
cp -a "$iris_src/src" "$rootfs/opt/archie/iris/src"
find "$rootfs/opt/archie/iris/src" -name __pycache__ -type d -prune -exec rm -rf {} +
install -m 0644 "$iris_src/requirements-iris.txt" "$iris_src/LICENSE" "$iris_src/MODEL_CARD.md" "$rootfs/opt/archie/iris/"

# Where torch.hub looks first, so IRIS starts without fetching code from GitHub.
hub="$rootfs/opt/archie/torch/hub/facebookresearch_vjepa2_$vjepa_revision"
install -d "$hub"
cp -a "$vjepa_src/." "$hub/"
rm -rf "${hub:?}/.git" "${hub:?}/notebooks" "${hub:?}/assets" "${hub:?}/tests"

arch-chroot "$rootfs" useradd --uid 1000 --create-home --shell /bin/bash archie
arch-chroot "$rootfs" passwd -l archie
install -d -o 1000 -g 1000 "$rootfs/var/lib/archie/models/iris" "$rootfs/opt/archie/iris-venv"
chown -R 1000:1000 "$rootfs/opt/archie/torch"
printf '%s\n' 'ArchieOS Operator. First time: archie-setup. Then: archie-operate "what to do" --app car-soccer' > "$rootfs/etc/motd"

arch-chroot "$rootfs" pacman -Q > "$rootfs/package-versions.txt"
rm -rf "$rootfs/var/cache/pacman/pkg"/* "$rootfs/var/log"/* "$rootfs/usr/share/doc/qt6" "$rootfs/usr/share/gtk-doc"
: > "$rootfs/etc/machine-id"

# Every binary finds its libraries inside the image, or the image is not shipped.
for binary in archie-stage archie-arena-live archie-stage-cli archie-ask archie-lan archie-netwatch archie-print; do
  if arch-chroot "$rootfs" ldd "/usr/bin/$binary" | grep -q 'not found'; then
    echo "build-operator: $binary lacks a library in the image" >&2
    arch-chroot "$rootfs" ldd "/usr/bin/$binary" | grep 'not found' >&2
    exit 65
  fi
done

du -sh "$rootfs"
if [[ $out == - ]]; then
  tar --numeric-owner --xattrs --acls -C "$rootfs" -cf - . | gzip -n -6 >&3
  exit 0
fi
out_dir=$(realpath -m "$out")
install -d "$out_dir"
tar --numeric-owner --xattrs --acls -C "$rootfs" -cf - . | gzip -n -6 > "$out_dir/ArchieOS-Operator.wsl"
( cd "$out_dir" && sha256sum ArchieOS-Operator.wsl > SHA256SUMS )
cp "$repo_root/scripts/wsl/install-operator.ps1" "$out_dir/install-archie-operator.ps1"
du -sh "$out_dir/ArchieOS-Operator.wsl"
