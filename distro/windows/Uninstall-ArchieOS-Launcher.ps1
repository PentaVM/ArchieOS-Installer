# Removes the ArchieOS launcher for this Windows user: its shortcuts (desktop, Start menu,
# Startup), its folder in %LOCALAPPDATA%, its settings and its entry in Settings → Apps.
#
#   .\Uninstall-ArchieOS-Launcher.ps1 [-Quiet] [-RemoveDistro NAME]
#
# The WSL distribution, the node inside it and its models stay unless -RemoveDistro names
# the distribution, and then it is asked once more: `wsl --unregister` deletes everything
# in it and cannot be undone.
param(
    [switch]$Quiet,
    [string]$RemoveDistro = ''
)
$ErrorActionPreference = 'Continue'
$target = Join-Path $env:LOCALAPPDATA 'ArchieOS\Launcher'

if (-not $Quiet) {
    $answer = Read-Host 'Remove the ArchieOS launcher, its shortcuts and its settings? [y/N]'
    if ($answer -notmatch '^[yYsS]') { Write-Host 'Nothing removed.'; exit 0 }
}

# The launcher only: this script's own name also ends in "ArchieOS-Launcher.ps1".
Get-CimInstance Win32_Process -Filter "Name='powershell.exe'" -ErrorAction SilentlyContinue |
    Where-Object { $_.ProcessId -ne $PID -and $_.CommandLine -match '[\\/"\s]ArchieOS-Launcher\.ps1' } |
    ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }

foreach ($folder in 'Desktop', 'Programs', 'Startup') {
    Remove-Item (Join-Path ([Environment]::GetFolderPath($folder)) 'ArchieOS.lnk') -ErrorAction SilentlyContinue
}
Remove-Item 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\ArchieOS-Launcher' -Recurse -ErrorAction SilentlyContinue
Remove-Item (Join-Path $env:APPDATA 'ArchieOS') -Recurse -Force -ErrorAction SilentlyContinue

if ($RemoveDistro) {
    $answer = Read-Host "Also delete the WSL distribution $RemoveDistro, with the node and every model in it? This cannot be undone. Type its name to confirm"
    if ($answer -eq $RemoveDistro) { wsl.exe --unregister $RemoveDistro } else { Write-Host "$RemoveDistro stays." }
}

# This script runs from the folder it removes: the folder goes once it has exited.
Start-Process cmd.exe -WindowStyle Hidden -ArgumentList '/d', '/c', "timeout /t 2 /nobreak >nul & rmdir /s /q `"$target`""
Write-Host 'ArchieOS launcher removed. The WSL distribution and its models stay (wsl --list shows it).'
