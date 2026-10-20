# Wanda LNCM on this PC: reads the Nebius key from the desktop file and runs the WSL script.
#
#   powershell -ExecutionPolicy Bypass -File training\wanda_lncm\train-local.ps1
#   powershell -ExecutionPolicy Bypass -File training\wanda_lncm\train-local.ps1 -Distro ArchieOS -BudgetUsd 22
#
# The key goes into this process's environment and through WSLENV to the script only. It is not
# printed, written to disk or committed.
param(
    [string]$Distro = "",
    [double]$BudgetUsd = 22,
    [string]$Stages = "data distill train",
    [string]$Preset = "small"
)
$ErrorActionPreference = "Stop"

$desktops = @(
    [Environment]::GetFolderPath("Desktop"),
    (Join-Path $env:USERPROFILE "Desktop"),
    (Join-Path $env:USERPROFILE "Escritorio"),
    (Join-Path $env:USERPROFILE "OneDrive\Desktop"),
    (Join-Path $env:USERPROFILE "OneDrive\Escritorio")
) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -Unique

$file = $desktops | ForEach-Object {
    Get-ChildItem -Path $_ -File -Filter "*.txt" -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -match "nebius" -and $_.Name -match "api" }
} | Select-Object -First 1
if (-not $file) { throw "No *nebius*api*.txt file on the desktop." }

# The longest key-shaped token in the file.
$key = (Select-String -Path $file.FullName -Pattern '[A-Za-z0-9._~+/=-]{20,}' -AllMatches).Matches |
    ForEach-Object { $_.Value } | Sort-Object Length -Descending | Select-Object -First 1
if (-not $key) { throw "No key found in $($file.Name)." }
Write-Host "Nebius key read from $($file.Name) (not shown)."

$env:NEBIUS_TOKEN_FACTORY_KEY = $key
$env:WANDA_BUDGET_USD = "$BudgetUsd"
$env:STAGES = $Stages
$env:WANDA_PRESET = $Preset
$shared = "NEBIUS_TOKEN_FACTORY_KEY/u:WANDA_BUDGET_USD/u:STAGES/u:WANDA_PRESET/u"
$env:WSLENV = if ($env:WSLENV) { "$($env:WSLENV):$shared" } else { $shared }

$repo = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$distroArgs = @()
if ($Distro) { $distroArgs = @("-d", $Distro) }
$wslRepo = (wsl.exe @distroArgs wslpath -a ($repo -replace '\\', '/')).Trim()
$wslArgs = $distroArgs + @("--", "bash", "-lc", "cd '$wslRepo' && training/wanda_lncm/train-local.sh")
try {
    wsl.exe @wslArgs
} finally {
    Remove-Item Env:NEBIUS_TOKEN_FACTORY_KEY -ErrorAction SilentlyContinue
}
