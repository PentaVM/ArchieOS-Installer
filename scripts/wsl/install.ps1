[CmdletBinding()]
param(
  [string]$Name = "ArchieOS",
  [string]$InstallLocation = "$env:LOCALAPPDATA\ArchieOS",
  [string]$ArtifactDirectory = $PSScriptRoot
)
$ErrorActionPreference = "Stop"
$modern = Join-Path $ArtifactDirectory "ArchieOS.wsl"
$fallback = Join-Path $ArtifactDirectory "ArchieOS-rootfs.tar.zst"
if (Test-Path -LiteralPath $modern) {
  & wsl.exe --install --from-file $modern
  if ($LASTEXITCODE -eq 0) { exit 0 }
}
if (-not (Test-Path -LiteralPath $fallback)) { throw "No ArchieOS rootfs artifact found" }
$zstd = Get-Command zstd.exe -ErrorAction SilentlyContinue
if (-not $zstd) { throw "Fallback import requires zstd.exe in PATH" }
New-Item -ItemType Directory -Force -Path $InstallLocation | Out-Null
$tar = Join-Path $env:TEMP "ArchieOS-rootfs-$PID.tar"
try {
  & $zstd.Source -d -f $fallback -o $tar
  if ($LASTEXITCODE -ne 0) { throw "zstd decompression failed" }
  & wsl.exe --import $Name $InstallLocation $tar --version 2
  if ($LASTEXITCODE -ne 0) { throw "wsl --import failed" }
} finally {
  Remove-Item -LiteralPath $tar -Force -ErrorAction SilentlyContinue
}

