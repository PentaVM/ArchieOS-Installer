@echo off
rem ArchieOS setup: double-click. Installs the launcher for this Windows user (no administrator
rem rights; Settings > Apps uninstalls it) and opens it on Install, which checks this PC, offers
rem to install WSL when it is missing, downloads the signed image and sets ArchieOS up.
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0Install-ArchieOS-Launcher.ps1"
if errorlevel 1 (
  echo.
  echo The ArchieOS launcher could not be installed. See the message above.
  pause
  exit /b 1
)
start "" wscript.exe "%LOCALAPPDATA%\ArchieOS\Launcher\ArchieOS.vbs" -Page instalar
