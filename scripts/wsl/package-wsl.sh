#!/usr/bin/env bash
set -euo pipefail

rootfs=${1:?usage: package-wsl.sh ROOTFS [OUT_DIR]}
out_dir=${2:-dist}
rootfs=$(realpath "$rootfs")
mkdir -p "$out_dir"
out_dir=$(realpath "$out_dir")

"$(dirname "$0")/validate-rootfs.sh" "$rootfs"
rm -f "$out_dir/ArchieOS.wsl" "$out_dir/ArchieOS-rootfs.tar.zst" "$out_dir/SHA256SUMS"

# Modern WSL packages are gzip-compressed rootfs tar archives with a .wsl suffix.
tar --numeric-owner --xattrs --acls -C "$rootfs" -cf - . | gzip -n -9 > "$out_dir/ArchieOS.wsl"
# Keep a zstd fallback; install.ps1 expands it before invoking wsl --import.
# A single worker at level 10 stays within the RAM available to the reference WSL
# builder. Higher levels caused swap thrashing without changing package semantics.
tar --numeric-owner --xattrs --acls -C "$rootfs" -cf - . | zstd -T1 -10 -o "$out_dir/ArchieOS-rootfs.tar.zst"
(
  cd "$out_dir"
  sha256sum ArchieOS.wsl ArchieOS-rootfs.tar.zst > SHA256SUMS
)

source_sha=${ARCHIE_SOURCE_SHA:-$(git -C "$(dirname "$0")/../.." rev-parse HEAD)}
cat > "$out_dir/build-metadata.json" <<EOF
{
  "format": 1,
  "git_sha": "$source_sha",
  "arch": "$(uname -m)",
  "built_at_utc": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "base": "Arch Linux",
  "packages": "package-versions.txt"
}
EOF
cp "$(dirname "$0")/install.ps1" "$out_dir/install-archie-wsl.ps1"
