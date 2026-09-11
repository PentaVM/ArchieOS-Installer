#!/usr/bin/env bash
# Build-only repository: exact reviewed extensions, original package signatures.
set -euo pipefail
[[ $# == 1 ]] || exit 64
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
output=$1
mkdir -p -- "$output"
output=$(cd -- "$output" && pwd)
packages=()
while read -r digest package url; do
  [[ -z "$digest" || "$digest" == \#* ]] && continue
  [[ "$digest" =~ ^[a-f0-9]{64}$ && "$package" =~ ^[a-z0-9._+-]+\.pkg\.tar\.zst$ && "$url" == https://archive.archlinux.org/packages/*/"$package" ]] || exit 65
  cached="/var/cache/pacman/pkg/$package"
  if [[ -f "$cached" ]]; then cp -- "$cached" "$output/$package";
  else curl --fail --location --proto '=https' --output "$output/$package" "$url"; fi
  printf '%s  %s\n' "$digest" "$output/$package" | sha256sum --check --status
  if [[ -f "$cached.sig" ]]; then cp -- "$cached.sig" "$output/$package.sig";
  else curl --fail --location --proto '=https' --output "$output/$package.sig" "$url.sig"; fi
  pacman-key --verify "$output/$package.sig" "$output/$package"
  packages+=("$output/$package")
done <"$root/arch-packages.lock"
[[ ${#packages[@]} == 2 ]] || exit 65
repo-add "$output/archie-reviewed.db.tar.gz" "${packages[@]}"
