# As administrator: gives back to Windows the space ArchieOS's WSL disk holds and no longer
# uses.
#
#   .\Compact-ArchieOS-Disk.ps1 [-Distro NAME]
#
# A WSL distribution's ext4.vhdx grows when Linux writes and does not shrink when it deletes:
# on 2026-10-24 it held 188 GB of C: with ~88 GB in use, C: reached 0 bytes and WSL hung so
# hard that its service would not stop. This trims the free space inside (fstrim marks it
# unused in the virtual disk), stops WSL and compacts the disk with diskpart. A sparse disk
# (wsl --manage --set-sparse true) is meant to give space back by itself, but on Windows 10
# it did not (923 GiB trimmed, nothing returned) and diskpart refuses to compact one, so the
# disk is made non-sparse first. The ArchieOS launcher calls it from "Liberar espacio" and
# brings the node back up after.
param([string]$Distro = '')
$ErrorActionPreference = 'Stop'
if (-not ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Run it as administrator.'
}
# Where WSL keeps the distribution, from its own registry entry.
$entries = Get-ChildItem 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Lxss' | ForEach-Object { Get-ItemProperty $_.PSPath }
if (-not $Distro) { $Distro = ($entries | Where-Object { $_.DistributionName -like 'ArchieOS*' } | Select-Object -First 1).DistributionName }
$entry = $entries | Where-Object { $_.DistributionName -eq $Distro } | Select-Object -First 1
if (-not $entry) { throw "No WSL distribution named $Distro." }
$vhdx = Join-Path ($entry.BasePath -replace '^\\\\\?\\', '') 'ext4.vhdx'
if (-not (Test-Path $vhdx)) { throw "No disk at $vhdx." }
$drive = (Get-Item $vhdx).PSDrive
$before = (Get-Item $vhdx).Length / 1GB
Write-Host ("{0}: {1:N1} GB, {2}: {3:N1} GB free" -f $vhdx, $before, $drive.Name, ($drive.Free / 1GB))

if (Get-Process vmmem -ErrorAction SilentlyContinue) {
    Write-Host 'Marking the free space inside as unused...'
    $trim = Start-Process wsl.exe -ArgumentList '-d', $Distro, '-u', 'root', '--', 'fstrim', '/' -PassThru -WindowStyle Hidden
    if (-not $trim.WaitForExit(300000)) { Stop-Process -Id $trim.Id -Force -ErrorAction SilentlyContinue }
}

Write-Host 'Stopping WSL...'
# A hung WSL does not answer --shutdown: it gets half a minute, then its service is stopped,
# and if the service does not stop either, only restarting Windows frees it.
$shutdown = Start-Process wsl.exe -ArgumentList '--shutdown' -PassThru -WindowStyle Hidden
if (-not $shutdown.WaitForExit(30000)) { Stop-Process -Id $shutdown.Id -Force -ErrorAction SilentlyContinue }
Start-Sleep 5
if (Get-Process vmmem -ErrorAction SilentlyContinue) {
    Write-Host 'WSL did not stop: stopping its service.'
    Get-Process wsl, wslhost, wslrelay -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    & sc.exe stop WSLService | Out-Null
    Stop-Process -Name wslservice -Force -ErrorAction SilentlyContinue
    for ($i = 0; $i -lt 30 -and (Get-Process vmmem -ErrorAction SilentlyContinue); $i++) { Start-Sleep 2 }
    if (Get-Process vmmem -ErrorAction SilentlyContinue) {
        throw 'WSL is still running and cannot be stopped: restart Windows, then run this before opening ArchieOS.'
    }
}

if ((Get-Item $vhdx).Attributes -band [IO.FileAttributes]::SparseFile) {
    Write-Host 'Making it non-sparse, so diskpart can compact it...'
    wsl.exe --manage $Distro --set-sparse false
}

Write-Host 'Compacting (several minutes)...'
$script = Join-Path $env:TEMP 'archie-compact.txt'
@"
select vdisk file="$vhdx"
attach vdisk readonly
compact vdisk
detach vdisk
exit
"@ | Set-Content $script -Encoding ASCII
diskpart /s $script
Remove-Item $script

$after = (Get-Item $vhdx).Length / 1GB
Write-Host ("Done: {0:N1} GB -> {1:N1} GB, {2}: {3:N1} GB free" -f $before, $after, $drive.Name, ((Get-PSDrive $drive.Name).Free / 1GB))
