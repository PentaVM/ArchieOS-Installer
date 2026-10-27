# Installs the ArchieOS Operator image into WSL.
#
#   .\install-archie-operator.ps1
#
# Run it from the folder that holds the image from the release: ArchieOS-Operator.wsl, or
# its parts ArchieOS-Operator.wsl.001, .002 ... (a release asset holds at most 2 GiB), and
# SHA256SUMS. IRIS, its weights and its text tower are inside the image: nothing else to
# download for it. It imports the image as the distribution "ArchieOS-Operator", makes the
# shared folder ArchieOS on the desktop, and starts the first-time setup, which installs
# the voice and asks for the Nebius key. Nothing is installed on Windows itself.
param(
    [string]$Name = 'ArchieOS-Operator',
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'ArchieOS-Operator')
)
$ErrorActionPreference = 'Stop'
$image = Join-Path $PSScriptRoot 'ArchieOS-Operator.wsl'
$sums = Join-Path $PSScriptRoot 'SHA256SUMS'

if ((wsl.exe --list --quiet) -replace "`0", '' | Where-Object { $_.Trim() -eq $Name }) {
    throw "A WSL distribution named $Name already exists. Remove it with: wsl --unregister $Name"
}
New-Item -ItemType Directory -Force $InstallDir | Out-Null
$parts = @(Get-ChildItem -Path $PSScriptRoot -Filter 'ArchieOS-Operator.wsl.*' |
    Where-Object { $_.Name -match '\.\d{3}$' } | Sort-Object Name)
$joined = $null
if (-not (Test-Path $image)) {
    if ($parts.Count -eq 0) { throw "Missing $image (or its parts). Download it from the release into this folder." }
    Write-Host "Joining the image from $($parts.Count) parts..."
    $joined = Join-Path $InstallDir 'ArchieOS-Operator.wsl'
    $out = [System.IO.File]::Create($joined)
    try {
        foreach ($part in $parts) {
            $in = [System.IO.File]::OpenRead($part.FullName)
            try { $in.CopyTo($out) } finally { $in.Dispose() }
        }
    } finally { $out.Dispose() }
    $image = $joined
}
if (Test-Path $sums) {
    Write-Host 'Checking the image...'
    $line = Get-Content $sums | Where-Object { $_ -match 'ArchieOS-Operator\.wsl$' } | Select-Object -First 1
    if (-not $line) { throw 'SHA256SUMS does not name ArchieOS-Operator.wsl.' }
    $expected = ($line -split '\s+')[0].ToLower()
    if ((Get-FileHash $image -Algorithm SHA256).Hash.ToLower() -ne $expected) {
        throw 'The image is damaged or incomplete. Download it (all its parts) again.'
    }
} else {
    Write-Host 'No SHA256SUMS beside the image: it is not checked.'
}

Write-Host "Importing $Name..."
New-Item -ItemType Directory -Force $InstallDir | Out-Null
wsl.exe --import $Name $InstallDir $image --version 2
if ($LASTEXITCODE -ne 0) { throw 'wsl --import failed. Is WSL 2 installed? (wsl --install)' }
if ($joined) { Remove-Item $joined }

# A freshly imported image has no local package-signing identity. Initialize it
# as root before user setup; --init preserves it if OOBE has already run.
wsl.exe -d $Name -u root -- /usr/lib/archie/init-pacman-keyring
if ($LASTEXITCODE -ne 0) { throw 'Package keyring initialization failed. Use a rebuilt image with the per-install initializer.' }

# The folder Archie and the person share: on the Windows desktop, and ~/ArchieOS inside.
$shared = Join-Path ([Environment]::GetFolderPath('Desktop')) 'ArchieOS'
New-Item -ItemType Directory -Force $shared | Out-Null
$inside = '/mnt/' + $shared.Substring(0, 1).ToLower() + ($shared.Substring(2) -replace '\\', '/')
# Quoted for the shell inside: a desktop path may hold spaces (OneDrive, a full name).
wsl.exe -d $Name -u archie -- bash -c "ln -sfn '$inside' /home/archie/ArchieOS"
Write-Host "Shared folder: $shared"

Write-Host 'First-time setup (the voice, about 1 GB)...'
wsl.exe -d $Name -u archie -- archie-setup
if ($LASTEXITCODE -ne 0) { throw "Setup did not finish. Run it again with: wsl -d $Name archie-setup" }

# A reasoning model on this machine, on the GPU: Bonsai 4B. Optional, because it adds
# about 8 GB (the CUDA toolkit) and a quarter of an hour of building.
Write-Host ''
$answer = Read-Host 'Install Bonsai, a reasoning model that runs on your GPU, so Archie answers without Nebius? [Y/n]'
if ($answer -notmatch '^[nN]') {
    wsl.exe -d $Name -u archie -- archie-local setup 4b
    wsl.exe -d $Name -u root -- archie-local gpu
    if ($LASTEXITCODE -ne 0) {
        Write-Host 'The GPU runtime did not build; Archie will use Nebius. Try again with:'
        Write-Host "  wsl -d $Name -u root archie-local gpu"
    }
}
Write-Host ''
Write-Host "Done. Open it with:  wsl -d $Name"
Write-Host 'Then:               archie-talk    (say "Archie" first)'
Write-Host '                    archie-video   (clips go in the ArchieOS folder on the desktop)'
