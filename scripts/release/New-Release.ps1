# Assembles an ArchieOS release for Windows from the built images: the release manifest
# (pinned to the Hugging Face revision the image was uploaded at), its signature, the
# installer zip and SHA256SUMS.
#
#   .\New-Release.ps1 -Version 0.1.0 -Image D:\...\ArchieOS-0.1.0.wsl -HfRepo PentaVM/ArchieOS `
#       -HfRevision <commit> -SourceCommit <commit> [-Ova D:\...\ArchieOS-0.1.0.ova] -Out D:\...\dist
#
# The manifest names exactly what the installer downloads (an https URL at a fixed revision, its
# size and SHA-256) and is signed with the release key (Sign-Release.ps1); the installer refuses
# it otherwise. Nothing is uploaded here.
param(
    [Parameter(Mandatory)][string]$Version,
    [Parameter(Mandatory)][string]$Image,
    [Parameter(Mandatory)][string]$HfRepo,
    [Parameter(Mandatory)][ValidatePattern('^[0-9a-f]{40}$')][string]$HfRevision,
    [Parameter(Mandatory)][ValidatePattern('^[0-9a-f]{40}$')][string]$SourceCommit,
    [string]$Ova = '',
    [Parameter(Mandatory)][string]$Out,
    [string]$Channel = 'alpha'
)
$ErrorActionPreference = 'Stop'
$repo = Resolve-Path (Join-Path $PSScriptRoot '..\..')
$windows = Join-Path $repo 'distro\windows'
New-Item -ItemType Directory -Force $Out | Out-Null

function Get-Sha([string]$path) { (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() }
$imageFile = Split-Path $Image -Leaf
$imageSha = Get-Sha $Image
$tag = "v$Version"

# The template keeps the targets the installer knows (IRIS embedded in the image, limits);
# only what this release pins changes.
$manifest = Get-Content (Join-Path $windows 'archieos-manifest.json') -Raw -Encoding UTF8 | ConvertFrom-Json
$manifest.release = $Version
$manifest.channel = $Channel
$manifest.provenance = [ordered]@{
    release_page   = "https://github.com/PentaVM/ArchieOS-Installer/releases/tag/$tag"
    source_commit  = $SourceCommit
    generated_by   = 'scripts/release/New-Release.ps1 from scripts/wsl/build-offline.sh'
    signature      = 'archieos-manifest.json.sig (ECDSA P-256, scripts/release/Sign-Release.ps1)'
}
$target = $manifest.targets.'windows-x86_64-wsl2'
$target.distro_name = 'ArchieOS'
$operator = $target.artifacts | Where-Object { $_.id -eq 'operator' }
$operator.file = $imageFile
$operator.source = "https://huggingface.co/$HfRepo/resolve/$HfRevision/$imageFile"
$operator.version = "$Version (source $($SourceCommit.Substring(0, 7)), revision $HfRevision)"
$operator.size_bytes = (Get-Item -LiteralPath $Image).Length
$operator.sha256 = $imageSha
# The repository is public: anyone downloads it, no token.
$operator.PSObject.Properties.Remove('auth')
$unavailable = @($manifest.unavailable | Where-Object { $_.id -notmatch 'Operator.wsl|virtualbox' })
$manifest.unavailable = $unavailable

# The signature covers these exact bytes: UTF-8 without a byte order mark, LF line ends.
$json = ($manifest | ConvertTo-Json -Depth 10) -replace "`r`n", "`n"
$manifestPath = Join-Path $Out 'archieos-manifest.json'
[IO.File]::WriteAllText($manifestPath, $json + "`n", (New-Object Text.UTF8Encoding($false)))
& (Join-Path $PSScriptRoot 'Sign-Release.ps1') sign -Manifest $manifestPath
& (Join-Path $PSScriptRoot 'Sign-Release.ps1') verify -Manifest $manifestPath

# The installer zip: the launcher, its engine and texts, the setup entry and the signed manifest.
$stage = Join-Path $Out "ArchieOS-Setup-$Version"
Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $stage | Out-Null
$files = 'Setup-ArchieOS.cmd', 'Install-ArchieOS-Launcher.ps1', 'Uninstall-ArchieOS-Launcher.ps1', 'ArchieOS-Launcher.ps1',
    'ArchieOS-Installer.ps1', 'ArchieOS-i18n.ps1', 'ArchieOS-Machine.ps1', 'ArchieOS-HostModel.ps1',
    'Compact-ArchieOS-Disk.ps1', 'ArchieOS.vbs', 'INSTALLER.md', 'INSTALLER.es.md'
foreach ($f in $files) { Copy-Item (Join-Path $windows $f) $stage }
# cmd.exe reads batch files line by line: they travel with CRLF.
$cmd = Join-Path $stage 'Setup-ArchieOS.cmd'
[IO.File]::WriteAllText($cmd, (([IO.File]::ReadAllText($cmd) -replace "`r`n", "`n") -replace "`n", "`r`n"), (New-Object Text.ASCIIEncoding))
Copy-Item $manifestPath, "$manifestPath.sig" $stage
Copy-Item (Join-Path $repo 'LICENSE') $stage
$zip = Join-Path $Out "ArchieOS-Setup-$Version.zip"
Remove-Item $zip -ErrorAction SilentlyContinue
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip
Remove-Item -Recurse -Force $stage

$sums = @("$(Get-Sha $zip)  $(Split-Path $zip -Leaf)", "$imageSha  $imageFile")
if ($Ova) { $sums += "$(Get-Sha $Ova)  $(Split-Path $Ova -Leaf)" }
$sums += "$(Get-Sha $manifestPath)  archieos-manifest.json"
[IO.File]::WriteAllText((Join-Path $Out 'SHA256SUMS'), (($sums -join "`n") + "`n"), (New-Object Text.UTF8Encoding($false)))
Get-Content (Join-Path $Out 'SHA256SUMS')
