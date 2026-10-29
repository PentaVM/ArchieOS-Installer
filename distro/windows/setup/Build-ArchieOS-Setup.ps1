# Builds ArchieOS-Setup.exe with the C# compiler that comes with Windows (.NET Framework 4,
# %WINDIR%\Microsoft.NET\Framework64\v4.0.30319\csc.exe): nothing to install. build.sh, beside
# this file, makes the same exe on Linux.
#
#   .\Build-ArchieOS-Setup.ps1 [-Out PATH] [-Manifest PATH]
#
# -Out defaults to dist\ArchieOS-Setup.exe. -Manifest swaps in another signed release manifest,
# with its .sig beside it, as New-Release.ps1 does with the one it has just signed. The files the
# exe carries are listed in payload.txt.
param(
    [string]$Out = '',
    [string]$Manifest = ''
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
if (-not $Out) { $Out = Join-Path $repo 'dist\ArchieOS-Setup.exe' }
$csc = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
if (-not (Test-Path -LiteralPath $csc)) { $csc = Join-Path $env:WINDIR 'Microsoft.NET\Framework\v4.0.30319\csc.exe' }
if (-not (Test-Path -LiteralPath $csc)) { throw "No C# compiler at $csc (.NET Framework 4)." }

$cscArgs = @('/nologo', '/noconfig', '/target:winexe', '/platform:anycpu', '/optimize+', '/codepage:65001',
    '/reference:System.dll', '/reference:System.Drawing.dll', '/reference:System.Windows.Forms.dll',
    "/win32icon:$(Join-Path $repo 'display\shell\brand\archieos.ico')", "/win32manifest:$(Join-Path $PSScriptRoot 'app.manifest')")
foreach ($line in Get-Content -LiteralPath (Join-Path $PSScriptRoot 'payload.txt')) {
    $line = $line.Trim()
    if (-not $line -or $line.StartsWith('#')) { continue }
    $parts = @($line -split '\s+')
    $file = Join-Path $repo ($parts[0] -replace '/', '\')
    $name = if ($parts.Count -gt 1) { $parts[1] } else { Split-Path $file -Leaf }
    if ($Manifest -and $name -eq 'archieos-manifest.json') { $file = $Manifest }
    if ($Manifest -and $name -eq 'archieos-manifest.json.sig') { $file = "$Manifest.sig" }
    if (-not (Test-Path -LiteralPath $file)) { throw "Missing $file" }
    $cscArgs += "/resource:$file,payload/$name"
}
New-Item -ItemType Directory -Force (Split-Path $Out) | Out-Null
& $csc @cscArgs "/out:$Out" (Join-Path $PSScriptRoot 'ArchieOSSetup.cs')
if ($LASTEXITCODE) { throw "csc failed ($LASTEXITCODE)" }
"{0}  {1}" -f (Get-FileHash -LiteralPath $Out -Algorithm SHA256).Hash.ToLowerInvariant(), $Out
