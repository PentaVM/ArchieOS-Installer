# Installs the ArchieOS Operator image into WSL.
#
#   .\install-archie-operator.ps1
#
# Run it from the folder that holds the image from the release: ArchieOS-Operator.wsl, or
# its parts ArchieOS-Operator.wsl.001, .002 ... (a release asset holds at most 2 GiB), and
# SHA256SUMS. IRIS, its weights and its text tower are inside the image: nothing else to
# download for it. It imports the image as the distribution "ArchieOS-Operator", makes the
# shared folder ArchieOS on the desktop, and starts the first-time setup, which installs
# the voice and asks for the Nebius key. On Windows it installs only the launcher, in this
# user's profile: the ArchieOS shortcuts, the start at sign-in (which waits a few seconds
# with a "No iniciar" button, and is turned off in the launcher or with -NoAutostart) and
# its entry in Settings → Apps, from where it is uninstalled.
#
# Install-ArchieOS.ps1, the installer with a window, runs this with -Unattended: no
# questions, the model it chose fetched and started, and the launcher left to it.
param(
    [string]$Name = 'ArchieOS-Operator',
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'ArchieOS-Operator'),
    [switch]$NoAutostart,
    # Where the image is: this script's folder unless said otherwise.
    [string]$Source = $PSScriptRoot,
    # A local model to fetch and start (archie-local list names them).
    [string]$Model = '',
    [switch]$Unattended,
    [switch]$NoLauncher
)
$ErrorActionPreference = 'Stop'
$image = Join-Path $Source 'ArchieOS-Operator.wsl'
$sums = Join-Path $Source 'SHA256SUMS'

# WSL 2 first: without it nothing below can work, and its own installer says what to do.
& wsl.exe --status *> $null
if ($LASTEXITCODE -ne 0) {
    throw 'WSL is not installed. In a PowerShell opened as administrator run: wsl --install   then restart Windows and run this again.'
}
if ((wsl.exe --list --quiet) -replace "`0", '' | Where-Object { $_.Trim() -eq $Name }) {
    throw "A WSL distribution named $Name already exists. Remove it with: wsl --unregister $Name"
}
New-Item -ItemType Directory -Force $InstallDir | Out-Null
# The image unpacks to several times its size, and the virtual disk only grows: a drive
# filled to the last byte froze WSL on the machine this was built on.
$drive = (Get-Item $InstallDir).PSDrive
if ($drive.Free -lt 40GB -and $Unattended) {
    Write-Warning ("{0}: has {1:N0} GB free; ArchieOS wants at least 40 GB." -f $drive.Name, ($drive.Free / 1GB))
} elseif ($drive.Free -lt 40GB) {
    $answer = Read-Host ("{0}: has {1:N0} GB free; ArchieOS wants at least 40 GB, plus room for its models. Continue anyway? [y/N]" -f $drive.Name, ($drive.Free / 1GB))
    if ($answer -notmatch '^[yYsS]') { throw 'Not installed. Free some space, or choose another drive with -InstallDir.' }
}
$parts = @(Get-ChildItem -Path $Source -Filter 'ArchieOS-Operator.wsl.*' |
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

# The folder Archie and the person share: on the Windows desktop, and ~/ArchieOS inside.
$shared = Join-Path ([Environment]::GetFolderPath('Desktop')) 'ArchieOS'
New-Item -ItemType Directory -Force $shared | Out-Null
$inside = '/mnt/' + $shared.Substring(0, 1).ToLower() + ($shared.Substring(2) -replace '\\', '/')
# Quoted for the shell inside: a desktop path may hold spaces (OneDrive, a full name).
wsl.exe -d $Name -u archie -- bash -c "ln -sfn '$inside' /home/archie/ArchieOS"
Write-Host "Shared folder: $shared"

Write-Host 'First-time setup (the voice, about 1 GB)...'
# Unattended, the Nebius key is not asked for here: the installer's window passes it after,
# if one was given, and the local model answers without it.
if ($Unattended) { wsl.exe -d $Name -u archie -- env ARCHIE_SETUP_NO_KEY=1 archie-setup }
else { wsl.exe -d $Name -u archie -- archie-setup }
if ($LASTEXITCODE -ne 0) { throw "Setup did not finish. Run it again with: wsl -d $Name archie-setup" }

# A reasoning model on this machine, on the GPU: PrismML's runtime and NVIDIA's CUDA
# libraries, about 800 MB to download. Which model is chosen in the launcher's model
# wizard, which says what fits this machine's GPU, memory and disk, or by -Model.
$hasNvidia = Test-Path (Join-Path $env:WINDIR 'System32\nvidia-smi.exe')
if ($Unattended) { $gpu = $hasNvidia }
else {
    Write-Host ''
    $answer = Read-Host 'Install the GPU runtime for a reasoning model on this machine, so Archie answers without Nebius? [Y/n]'
    $gpu = $answer -notmatch '^[nN]'
}
if ($gpu) {
    wsl.exe -d $Name -u root -- archie-local gpu
    if ($LASTEXITCODE -ne 0) {
        Write-Host 'The GPU runtime did not install; Archie will use Nebius. Try again with:'
        Write-Host "  wsl -d $Name -u root archie-local gpu"
        $gpu = $false
    }
}
if ($Model) {
    Write-Host "Fetching $Model..."
    wsl.exe -d $Name -u archie -- archie-local setup $Model
    if ($LASTEXITCODE -ne 0) { throw "$Model did not arrive. Try again with: wsl -d $Name archie-local setup $Model" }
    wsl.exe -d $Name -u archie -- archie-local use $Model
}

# The launcher: Start/Restart, the GUI, the voice and the model, and the node up at sign-in.
# Beside this script in a release; in a checkout of the repository, in distro/windows.
$launcher = @(
    (Join-Path $PSScriptRoot 'Install-ArchieOS-Launcher.ps1'),
    (Join-Path $PSScriptRoot '..\..\distro\windows\Install-ArchieOS-Launcher.ps1')
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if ($NoLauncher) {
    $launcher = $null
} elseif ($launcher) {
    $options = @{ Distro = $Name }
    if ($gpu -and -not $Model) { $options.OpenModelWizard = $true }
    if ($NoAutostart) { $options.NoAutostart = $true }
    & $launcher @options
} elseif ($gpu -and -not $Model) {
    # Without the launcher beside this script, the model that fits an 8 GB card whole.
    wsl.exe -d $Name -u archie -- archie-local setup 9b
}
Write-Host ''
if ($NoAutostart) { Write-Host 'Done. The ArchieOS icon on the desktop starts it.' }
else { Write-Host 'Done. The ArchieOS icon on the desktop starts it, and it starts at sign-in (the launcher turns that off).' }
Write-Host "Or open it with:     wsl -d $Name"
Write-Host 'Then:               archie-talk    (say "Archie" first)'
Write-Host '                    archie-video   (clips go in the ArchieOS folder on the desktop)'
