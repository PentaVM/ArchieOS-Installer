# Installs the ArchieOS launcher for this Windows user: copies it to
# %LOCALAPPDATA%\ArchieOS\Launcher, makes the shortcuts "ArchieOS" on the desktop and in the
# Start menu, and one in the Startup folder that brings the node up at sign-in, and lists it
# in Settings → Apps, from where it is uninstalled like any other application.
#
#   .\Install-ArchieOS-Launcher.ps1 [-Distro NAME] [-NoAutostart] [-OpenModelWizard]
#
# Nothing goes outside this user's profile, and no administrator rights are needed. Run it
# again to update; Uninstall-ArchieOS-Launcher.ps1 (or Settings → Apps) removes it.
param(
    [string]$Distro = '',
    [switch]$NoAutostart,
    [switch]$OpenModelWizard
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$version = '0.1.0'
$target = Join-Path $env:LOCALAPPDATA 'ArchieOS\Launcher'
New-Item -ItemType Directory -Force $target | Out-Null

# A launcher already running keeps the old script open: it is asked to close first.
Get-CimInstance Win32_Process -Filter "Name='powershell.exe'" -ErrorAction SilentlyContinue |
    Where-Object { $_.CommandLine -like '*ArchieOS-Launcher.ps1*' } |
    ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }

# Windows PowerShell 5.1 reads a script without a byte order mark as ANSI: each copy is
# written as UTF-8 with one, so the accents of its Spanish text survive.
$utf8Bom = New-Object System.Text.UTF8Encoding($true)
foreach ($script in 'ArchieOS-Launcher.ps1', 'Compact-ArchieOS-Disk.ps1', 'Uninstall-ArchieOS-Launcher.ps1') {
    $source = Get-Content (Join-Path $PSScriptRoot $script) -Raw -Encoding UTF8
    [System.IO.File]::WriteAllText((Join-Path $target $script), $source, $utf8Bom)
}
Copy-Item (Join-Path $PSScriptRoot 'ArchieOS.vbs') $target -Force

# The icon: Archie's orb, drawn here rather than shipped as a binary. A 256-pixel PNG inside
# an .ico, which Windows scales for the taskbar, the tray and the shortcuts.
function New-OrbIcon([string]$path) {
    $size = 256
    $bmp = New-Object System.Drawing.Bitmap $size, $size
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = 'AntiAlias'
    $g.Clear([System.Drawing.Color]::Transparent)
    $orb = New-Object System.Drawing.Drawing2D.GraphicsPath
    $orb.AddEllipse(16, 16, $size - 32, $size - 32)
    $fill = New-Object System.Drawing.Drawing2D.PathGradientBrush $orb
    $fill.CenterPoint = New-Object System.Drawing.PointF 104, 96
    $fill.CenterColor = [System.Drawing.Color]::FromArgb(255, 200, 245, 255)
    $fill.SurroundColors = @([System.Drawing.Color]::FromArgb(255, 10, 46, 110))
    $g.FillPath($fill, $orb)
    $ring = New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb(220, 120, 220, 255)), 6
    $g.DrawEllipse($ring, 16, 16, $size - 32, $size - 32)
    $g.Dispose()
    $png = New-Object System.IO.MemoryStream
    $bmp.Save($png, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    $bytes = $png.ToArray()
    $out = New-Object System.IO.MemoryStream
    $w = New-Object System.IO.BinaryWriter $out
    $w.Write([uint16]0); $w.Write([uint16]1); $w.Write([uint16]1)        # header: one image
    $w.Write([byte]0); $w.Write([byte]0); $w.Write([byte]0); $w.Write([byte]0)  # 256x256
    $w.Write([uint16]1); $w.Write([uint16]32)                           # planes, bits
    $w.Write([uint32]$bytes.Length); $w.Write([uint32]22)               # size, offset
    $w.Write($bytes)
    $w.Flush()
    [System.IO.File]::WriteAllBytes($path, $out.ToArray())
}
$icon = Join-Path $target 'ArchieOS.ico'
New-OrbIcon $icon

if ($Distro) {
    $settingsDir = Join-Path $env:APPDATA 'ArchieOS'
    $settingsFile = Join-Path $settingsDir 'launcher.json'
    New-Item -ItemType Directory -Force $settingsDir | Out-Null
    $settings = if (Test-Path $settingsFile) { Get-Content $settingsFile -Raw -Encoding UTF8 | ConvertFrom-Json } else { [pscustomobject]@{} }
    $settings | Add-Member -NotePropertyName distro -NotePropertyValue $Distro -Force
    $settings | Add-Member -NotePropertyName backend -NotePropertyValue 'wsl' -Force
    $settings | ConvertTo-Json | Set-Content $settingsFile -Encoding UTF8
}

$shell = New-Object -ComObject WScript.Shell
function New-Link([string]$path, [string]$arguments, [string]$description) {
    $lnk = $shell.CreateShortcut($path)
    $lnk.TargetPath = Join-Path $env:WINDIR 'System32\wscript.exe'
    $lnk.Arguments = ('"{0}" {1}' -f (Join-Path $target 'ArchieOS.vbs'), $arguments).Trim()
    $lnk.WorkingDirectory = $target
    $lnk.IconLocation = "$icon,0"
    $lnk.Description = $description
    $lnk.Save()
}
New-Link (Join-Path ([Environment]::GetFolderPath('Desktop')) 'ArchieOS.lnk') '' 'ArchieOS: iniciar, reiniciar, configurar y abrir la GUI'
New-Link (Join-Path ([Environment]::GetFolderPath('Programs')) 'ArchieOS.lnk') '' 'ArchieOS: iniciar, reiniciar, configurar y abrir la GUI'
$startup = Join-Path ([Environment]::GetFolderPath('Startup')) 'ArchieOS.lnk'
# An update keeps the choice the person made in the launcher: the sign-in shortcut is made
# on a first install, refreshed if it is there, and left out if it was turned off.
$firstInstall = -not (Test-Path (Join-Path $env:APPDATA 'ArchieOS\installed'))
if ($NoAutostart) { Remove-Item $startup -ErrorAction SilentlyContinue }
elseif ($firstInstall -or (Test-Path $startup)) { New-Link $startup '-Autostart' 'ArchieOS: levanta el nodo al entrar a Windows' }
New-Item -ItemType Directory -Force (Join-Path $env:APPDATA 'ArchieOS') | Out-Null
Set-Content (Join-Path $env:APPDATA 'ArchieOS\installed') $version -Encoding ASCII

# Settings → Apps lists it, with its uninstaller, for this user only.
$key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\ArchieOS-Launcher'
New-Item -Path $key -Force | Out-Null
$uninstall = 'powershell.exe -NoProfile -ExecutionPolicy Bypass -File "{0}"' -f (Join-Path $target 'Uninstall-ArchieOS-Launcher.ps1')
$values = @{
    DisplayName = 'ArchieOS'; DisplayVersion = $version; Publisher = 'ArchieOS'
    DisplayIcon = $icon; InstallLocation = $target
    UninstallString = $uninstall; QuietUninstallString = "$uninstall -Quiet"
}
foreach ($name in $values.Keys) { Set-ItemProperty -Path $key -Name $name -Value $values[$name] }
Set-ItemProperty -Path $key -Name NoModify -Value 1 -Type DWord
Set-ItemProperty -Path $key -Name NoRepair -Value 1 -Type DWord
Set-ItemProperty -Path $key -Name EstimatedSize -Value 1024 -Type DWord

Write-Host "ArchieOS launcher $version installed in $target"
if ($OpenModelWizard) {
    Start-Process (Join-Path $env:WINDIR 'System32\wscript.exe') -ArgumentList ('"{0}" -Page modelo' -f (Join-Path $target 'ArchieOS.vbs'))
}
