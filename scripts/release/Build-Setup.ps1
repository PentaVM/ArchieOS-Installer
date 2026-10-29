# Builds ArchieOS-Setup-<version>.exe: distro/windows/setup/ArchieOSSetup.cs, compiled with the C#
# compiler of the .NET Framework that ships with Windows, with the launcher carried inside as
# resources: its scripts, texts, model catalog, the brand icon and the signed release manifest.
#
#   .\Build-Setup.ps1 -Manifest DIST\archieos-manifest.json -Out DIST [-Version 0.1.0]
#
# The manifest must be the signed one (its .sig beside it): the launcher refuses anything else.
param(
    [Parameter(Mandatory)][string]$Manifest,
    [Parameter(Mandatory)][string]$Out,
    [string]$Version = '0.1.0'
)
$ErrorActionPreference = 'Stop'
$repo = Resolve-Path (Join-Path $PSScriptRoot '..\..')
$windows = Join-Path $repo 'distro\windows'
$csc = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
if (-not (Test-Path $csc)) { throw "No C# compiler at $csc (.NET Framework 4.x)" }
if (-not (Test-Path "$Manifest.sig")) { throw "$Manifest has no signature beside it" }
New-Item -ItemType Directory -Force $Out | Out-Null

$payload = @(
    'Install-ArchieOS-Launcher.ps1', 'Uninstall-ArchieOS-Launcher.ps1', 'ArchieOS-Launcher.ps1', 'ArchieOS-Installer.ps1',
    'ArchieOS-i18n.ps1', 'ArchieOS-Machine.ps1', 'ArchieOS-HostModel.ps1', 'Compact-ArchieOS-Disk.ps1', 'ArchieOS.vbs'
) | ForEach-Object { Join-Path $windows $_ }
$payload += (Join-Path $repo 'distro\wsl-operator\rootfs\usr\share\archie\local-models.tsv')
$payload += (Join-Path $repo 'display\shell\brand\archieos.ico')
$payload += $Manifest, "$Manifest.sig"
$resources = $payload | ForEach-Object {
    if (-not (Test-Path -LiteralPath $_)) { throw "missing $_" }
    '/resource:"{0}",payload/{1}' -f $_, (Split-Path $_ -Leaf)
}
$exe = Join-Path $Out "ArchieOS-Setup-$Version.exe"
$argv = @('/nologo', '/target:winexe', '/optimize+', '/platform:anycpu',
    "/win32icon:$(Join-Path $repo 'display\shell\brand\archieos.ico')",
    '/reference:System.Windows.Forms.dll', "/out:$exe") + $resources + (Join-Path $windows 'setup\ArchieOSSetup.cs')
& $csc @argv
if ($LASTEXITCODE -ne 0) { throw "csc failed ($LASTEXITCODE)" }
$hash = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant()
'{0}  {1} ({2:N0} bytes)' -f $hash, (Split-Path $exe -Leaf), (Get-Item $exe).Length
