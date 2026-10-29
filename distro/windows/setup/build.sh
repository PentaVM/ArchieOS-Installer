#!/usr/bin/env bash
# Builds ArchieOS-Setup.exe on Linux: the .NET SDK's C# compiler (Roslyn) against the .NET
# Framework 4.8 reference assemblies, with the same arguments Build-ArchieOS-Setup.ps1 gives the
# compiler that comes with Windows. -deterministic: the same sources give the same exe.
#
#   distro/windows/setup/build.sh [OUT.exe] [MANIFEST.json]
#
# OUT defaults to dist/ArchieOS-Setup.exe. MANIFEST swaps in another signed release manifest,
# with its .sig beside it. Needs dotnet (any SDK), curl and python3; the reference assemblies
# come from NuGet once and are checked against their SHA-256.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
out="${1:-$repo/dist/ArchieOS-Setup.exe}"
manifest="${2:-}"

pkg=microsoft.netframework.referenceassemblies.net48
ver=1.0.3
sha=8a7e348538e7eb91351696911689f49e3d4f63f8bab517432bbe159b8b1104a2
cache="${XDG_CACHE_HOME:-$HOME/.cache}/archieos-setup/$pkg.$ver"
refs="$cache/build/.NETFramework/v4.8"
if [ ! -f "$refs/mscorlib.dll" ]; then
  mkdir -p "$cache"
  curl -fsSL -o "$cache.nupkg" "https://api.nuget.org/v3-flatcontainer/$pkg/$ver/$pkg.$ver.nupkg"
  echo "$sha  $cache.nupkg" | sha256sum -c --quiet
  python3 -c 'import sys, zipfile; zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])' "$cache.nupkg" "$cache"
fi

sdk="$(dotnet --list-sdks | tail -n 1)"
csc="$(echo "$sdk" | sed -E 's/^([^ ]+) \[(.*)\]$/\2\/\1/')/Roslyn/bincore/csc.dll"
[ -f "$csc" ] || { echo "no csc.dll in $sdk" >&2; exit 1; }

args=(-nologo -noconfig -nostdlib+ -target:winexe -platform:anycpu -optimize+ -deterministic+
  -langversion:5 -codepage:65001
  "-reference:$refs/mscorlib.dll" "-reference:$refs/System.dll"
  "-reference:$refs/System.Drawing.dll" "-reference:$refs/System.Windows.Forms.dll"
  "-win32icon:$repo/display/shell/brand/archieos.ico" "-win32manifest:$here/app.manifest")
while read -r path name; do
  case "$path" in ''|'#'*) continue ;; esac
  name="${name:-$(basename "$path")}"
  file="$repo/$path"
  if [ -n "$manifest" ] && [ "$name" = archieos-manifest.json ]; then file="$manifest"; fi
  if [ -n "$manifest" ] && [ "$name" = archieos-manifest.json.sig ]; then file="$manifest.sig"; fi
  [ -f "$file" ] || { echo "missing $file" >&2; exit 1; }
  args+=("-resource:$file,payload/$name")
done < "$here/payload.txt"

mkdir -p "$(dirname "$out")"
dotnet "$csc" "${args[@]}" "-out:$out" "$here/ArchieOSSetup.cs"
sha256sum "$out"
