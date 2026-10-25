# The local model served from Windows, for an ArchieOS that runs in VirtualBox.
#
# VirtualBox cannot give a virtual machine the NVIDIA GPU on Windows: there is no CUDA
# inside it, and NVIDIA's driver is not something anyone else can extend. So the model runs
# where the GPU is, on Windows, with PrismML's own Windows build of the same llama.cpp
# ArchieOS runs inside WSL, and the machine reaches it through VirtualBox's private network
# (10.0.2.2 is Windows, seen from inside). It listens on Windows' loopback only, as the model
# in WSL does; inside the machine a systemd proxy puts it on the machine's own loopback,
# where Core accepts a local model. Dot-sourced by the installer and the launcher; needs
# ArchieOS-Machine.ps1 dot-sourced first.

# Downloads with curl.exe (part of Windows 10 1803 and later): resumable, and fast where
# Invoke-WebRequest is not. Every file is checked against its digest before it is used.
function Save-Verified([string]$url, [string]$path, [string]$sha256, [scriptblock]$log) {
    if ((Test-Path $path) -and (Get-FileHash $path -Algorithm SHA256).Hash -eq $sha256.ToUpper()) { return }
    New-Item -ItemType Directory -Force (Split-Path $path) | Out-Null
    & $log "Descargando $(Split-Path $path -Leaf)"
    & curl.exe --fail --location --silent --show-error --proto '=https' --tlsv1.2 --continue-at - --output "$path.part" $url
    if ($LASTEXITCODE -ne 0) { throw "No se pudo descargar $url" }
    if ((Get-FileHash "$path.part" -Algorithm SHA256).Hash -ne $sha256.ToUpper()) {
        Remove-Item "$path.part" -Force
        throw "$(Split-Path $path -Leaf) no llegó completo; vuelve a intentarlo"
    }
    Move-Item -Force "$path.part" $path
}

# PrismML's runtime for this machine's GPU, in %LOCALAPPDATA%\ArchieOS\runtime\<kind>.
function Install-HostRuntime([string]$kind, [scriptblock]$log) {
    $dir = Join-Path $env:LOCALAPPDATA "ArchieOS\runtime\$kind"
    if (Test-Path (Join-Path $dir 'llama-server.exe')) { return $dir }
    $cache = Join-Path $env:LOCALAPPDATA 'ArchieOS\downloads'
    foreach ($asset in $script:ArchieRuntimes[$kind]) {
        $zip = Join-Path $cache $asset.file
        Save-Verified "https://github.com/PrismML-Eng/llama.cpp/releases/download/$($script:ArchieRuntimeRelease)/$($asset.file)" $zip $asset.sha256 $log
        Expand-Archive -Path $zip -DestinationPath $dir -Force
        Remove-Item $zip
    }
    # Some releases put the files one folder down.
    $server = Get-ChildItem $dir -Recurse -Filter 'llama-server.exe' | Select-Object -First 1
    if (-not $server) { throw "El runtime $kind no trae llama-server.exe" }
    if ($server.DirectoryName -ne $dir) {
        Get-ChildItem $server.DirectoryName | Move-Item -Destination $dir -Force
    }
    return $dir
}

# A model of the catalog, all its parts, in $modelsDir. Returns the first part's path.
function Install-HostModel($entry, [string]$modelsDir, [scriptblock]$log) {
    $count = $entry.sha256.Count
    for ($i = 1; $i -le $count; $i++) {
        $path = $entry.file
        if ($count -gt 1) { $path = $path -replace '-00001-of-', ('-{0:D5}-of-' -f $i) }
        $url = "https://huggingface.co/$($entry.repo)/resolve/$($entry.rev)/$path"
        Save-Verified $url (Join-Path $modelsDir ($path -split '/')[-1]) $entry.sha256[$i - 1] $log
    }
    return Join-Path $modelsDir (($entry.file -split '/')[-1])
}

# The server's command line, as archie-local builds it inside WSL: the KV cache in q8_0, as
# many layers on the GPU as fit with 512 MB to spare.
function Get-HostModelArguments($hostModel) {
    $arguments = @('-m', "`"$($hostModel.path)`"", '--host', '127.0.0.1', '--port', $hostModel.port, '--alias', $hostModel.alias,
        '--jinja', '--ctx-size', '16384', '--parallel', '1')
    if ($hostModel.runtime -ne 'cpu') {
        $arguments += @('-ngl', 'auto', '--fit', 'on', '--fit-target', '512', '--flash-attn', 'on', '--cache-type-k', 'q8_0', '--cache-type-v', 'q8_0')
    }
    if ($hostModel.reasoning -eq 'effort') { $arguments += @('--reasoning-effort-allow', 'none,medium,xhigh', '--reasoning-effort-fallback', 'medium') }
    return $arguments
}

function Test-HostModel($hostModel) {
    try { return (Invoke-WebRequest -UseBasicParsing -TimeoutSec 2 "http://127.0.0.1:$($hostModel.port)/health").StatusCode -eq 200 } catch { return $false }
}

# Started with no window; its output goes to %LOCALAPPDATA%\ArchieOS\logs.
function Start-HostModel($hostModel) {
    if (Test-HostModel $hostModel) { return }
    $logs = Join-Path $env:LOCALAPPDATA 'ArchieOS\logs'
    New-Item -ItemType Directory -Force $logs | Out-Null
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = Join-Path $hostModel.runtime_dir 'llama-server.exe'
    $psi.Arguments = (Get-HostModelArguments $hostModel) -join ' '
    $psi.WorkingDirectory = $hostModel.runtime_dir
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    $psi.RedirectStandardOutput = $false
    $psi.RedirectStandardError = $false
    $p = [System.Diagnostics.Process]::Start($psi)
    Set-Content (Join-Path $logs 'host-model.pid') $p.Id
}

function Stop-HostModel {
    $pidFile = Join-Path $env:LOCALAPPDATA 'ArchieOS\logs\host-model.pid'
    if (Test-Path $pidFile) {
        Stop-Process -Id ([int](Get-Content $pidFile)) -Force -ErrorAction SilentlyContinue
        Remove-Item $pidFile -ErrorAction SilentlyContinue
    }
}

# The machine: imported from ArchieOS.ova with the resources chosen, the model's name in its
# guest properties (ArchieOS reads it when it starts and finds the rest in its catalog), its
# window full screen.
function New-ArchieVm([string]$ova, [string]$name, [int]$cpus, [int]$memoryGb, [string]$baseFolder, $hostModel, [scriptblock]$log) {
    $vbox = Join-Path $env:ProgramFiles 'Oracle\VirtualBox\VBoxManage.exe'
    & $log "Importando $name en VirtualBox"
    # VBoxManage writes progress to stderr, an error under 'Stop' in Windows PowerShell 5.1:
    # its exit code decides instead.
    $ErrorActionPreference = 'Continue'
    & $vbox import $ova --vsys 0 --vmname $name --cpus $cpus --memory ($memoryGb * 1024) --basefolder $baseFolder 2>&1 | ForEach-Object { & $log "$_" }
    if ($LASTEXITCODE -ne 0) { throw 'VirtualBox no pudo importar la máquina' }
    # 10.0.2.2 reaches Windows' loopback only with this on (off by default since 7.0).
    & $vbox modifyvm $name --nat-localhostreachable1 on --graphicscontroller vmsvga --vram 128 --accelerate-3d on `
        --clipboard-mode bidirectional --audio-enabled on --audio-in on --audio-out on 2>&1 | ForEach-Object { & $log "$_" }
    if ($hostModel) {
        & $vbox guestproperty set $name /ArchieOS/Model/Name $hostModel.name --flags RDONLYGUEST
    }
    & $vbox setextradata $name GUI/Fullscreen true
    & $vbox setextradata $name GUI/LastCloseAction PowerOff
}
