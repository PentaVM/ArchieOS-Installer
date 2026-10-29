# Serves Nemotron 3 Nano Omni (30B-A3B, hears, sees and reasons) from Windows, where all of the
# machine's memory is: the model is 18.5 GB and WSL is given 9. llama-server listens on
# Windows' loopback only, and omni-agent carries ArchieOS's requests to it from omni-bridge in
# WSL (connecting from Windows: no firewall rule, no WSL interop). Run it on Windows before
# `archie-talk --nemotron`; -Stop stops both.
#   powershell -ExecutionPolicy Bypass -File D:\ArchieLocal\scripts\serve-nemotron-omni.ps1
param(
    [string]$Root = 'D:\ArchieLocal',
    [int]$Port = 8111,
    [int]$Context = 16384,
    # MB of the GPU left for Windows, the desktop and the speech models.
    [int]$FitTargetMB = 1536,
    # The bridge's agent port in WSL (omni-bridge), and the Python that runs the agent.
    [int]$BridgePort = 8112,
    [string]$Python = 'C:\Python312\pythonw.exe',
    [switch]$Stop
)
$ErrorActionPreference = 'Stop'

function Get-OmniServer {
    Get-NetTCPConnection -LocalAddress 127.0.0.1 -LocalPort $Port -State Listen -ErrorAction SilentlyContinue |
        ForEach-Object { Get-Process -Id $_.OwningProcess -ErrorAction SilentlyContinue }
}

function Get-OmniAgent {
    Get-CimInstance Win32_Process -Filter "Name like 'python%'" |
        Where-Object { $_.CommandLine -like '*omni-agent.py*' }
}

if ($Stop) {
    Get-OmniServer | Stop-Process -Force
    Get-OmniAgent | ForEach-Object { Stop-Process -Id $_.ProcessId -Force }
    return
}
if (-not (Get-OmniAgent)) {
    $agent = Join-Path $PSScriptRoot 'omni-agent.py'
    Start-Process -FilePath $Python -ArgumentList "`"$agent`"", "$BridgePort", "$Port" -WindowStyle Hidden
}
if (Get-OmniServer) { return }

$runtime = Join-Path $env:LOCALAPPDATA 'ArchieOS\runtime\cuda-12.4\llama-server.exe'
$dir = Join-Path $Root 'models\nemotron-omni'
$model = Join-Path $dir 'NVIDIA-Nemotron-3-Nano-Omni-30B-A3B-Reasoning-UD-Q2_K_XL.gguf'
# ggml-org's projector: vision and the Parakeet audio encoder (other repositories ship vision only).
$mmproj = Join-Path $dir 'mmproj-NVIDIA-Nemotron-3-Nano-Omni-30B-A3B-Q8_0.gguf'
foreach ($f in $runtime, $model, $mmproj) {
    if (-not (Test-Path $f)) { throw "serve-nemotron-omni: missing $f" }
}
$logs = Join-Path $Root 'logs'
New-Item -ItemType Directory -Force $logs | Out-Null

# What fits goes on the GPU (the shared layers, the projector, the cache and as many experts as
# there is room for); the other experts stay in RAM and are read from the SSD as needed.
$arguments = @(
    '-m', "`"$model`"", '--mmproj', "`"$mmproj`"",
    '--host', '127.0.0.1', '--port', "$Port", '--alias', 'nemotron-omni', '--jinja',
    '-ngl', 'auto', '--fit', 'on', '--fit-target', "$FitTargetMB",
    '--ctx-size', "$Context", '--parallel', '1',
    '--flash-attn', 'on', '--cache-type-k', 'q8_0', '--cache-type-v', 'q8_0',
    '--cont-batching', '--cache-reuse', '256',
    '--threads', '8'
)
Start-Process -FilePath $runtime -ArgumentList $arguments -WindowStyle Hidden `
    -RedirectStandardOutput (Join-Path $logs 'nemotron-omni.out.log') `
    -RedirectStandardError (Join-Path $logs 'nemotron-omni.log')
