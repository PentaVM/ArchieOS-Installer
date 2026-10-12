# Installs the ArchieOS Operator image into WSL.
#
#   .\install-archie-operator.ps1
#
# Run it from the folder that holds ArchieOS-Operator.wsl and iris-beta21-step132000.pt
# (both from the release). It imports the image as the distribution "ArchieOS-Operator",
# puts IRIS's weights in it, makes the shared folder ArchieOS on the desktop, and starts the
# first-time setup, which installs PyTorch and the voice and asks for the Nebius key.
# Nothing is installed on Windows itself.
param(
    [string]$Name = 'ArchieOS-Operator',
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'ArchieOS-Operator')
)
$ErrorActionPreference = 'Stop'
$image = Join-Path $PSScriptRoot 'ArchieOS-Operator.wsl'
$weights = Join-Path $PSScriptRoot 'iris-beta21-step132000.pt'
$weightsSha = 'c6cadb02f5a5bffc4ea3c3348d32b475ef7531f643675c06c65451037e4f3b97'

foreach ($file in $image, $weights) {
    if (-not (Test-Path $file)) { throw "Missing $file. Download it from the release into this folder." }
}
if ((wsl.exe --list --quiet) -replace "`0", '' | Where-Object { $_.Trim() -eq $Name }) {
    throw "A WSL distribution named $Name already exists. Remove it with: wsl --unregister $Name"
}
Write-Host 'Checking the weights...'
if ((Get-FileHash $weights -Algorithm SHA256).Hash.ToLower() -ne $weightsSha) {
    throw 'iris-beta21-step132000.pt is damaged or is not IRIS Beta 2.1 step 132000. Download it again.'
}

Write-Host "Importing $Name..."
New-Item -ItemType Directory -Force $InstallDir | Out-Null
wsl.exe --import $Name $InstallDir $image --version 2
if ($LASTEXITCODE -ne 0) { throw 'wsl --import failed. Is WSL 2 installed? (wsl --install)' }

Write-Host 'Copying the weights into the image...'
$target = "\\wsl.localhost\$Name\var\lib\archie\models\iris"
wsl.exe -d $Name -u archie -- true
Copy-Item $weights (Join-Path $target 'iris-beta21-step132000.pt')

# The folder Archie and the person share: on the Windows desktop, and ~/ArchieOS inside.
$shared = Join-Path ([Environment]::GetFolderPath('Desktop')) 'ArchieOS'
New-Item -ItemType Directory -Force $shared | Out-Null
$inside = '/mnt/' + $shared.Substring(0, 1).ToLower() + ($shared.Substring(2) -replace '\\', '/')
# Quoted for the shell inside: a desktop path may hold spaces (OneDrive, a full name).
wsl.exe -d $Name -u archie -- bash -c "ln -sfn '$inside' /home/archie/ArchieOS"
Write-Host "Shared folder: $shared"

Write-Host 'First-time setup (PyTorch is about 3 GB, the voice about 1 GB)...'
wsl.exe -d $Name -u archie -- archie-setup
if ($LASTEXITCODE -ne 0) { throw "Setup did not finish. Run it again with: wsl -d $Name archie-setup" }
Write-Host ''
Write-Host "Done. Open it with:  wsl -d $Name"
Write-Host 'Then:               archie-talk    (say "Archie" first)'
Write-Host '                    archie-video   (clips go in the ArchieOS folder on the desktop)'
