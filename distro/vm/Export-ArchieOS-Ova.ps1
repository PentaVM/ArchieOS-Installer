# Makes ArchieOS.ova from the disk distro/vm/build-vm.sh built: a VirtualBox machine set
# up as ArchieOS runs (UEFI, VMSVGA with 3D, NAT that reaches Windows' loopback, audio),
# optionally booted once to check it, then exported with its SHA256SUMS line.
#
#   .\Export-ArchieOS-Ova.ps1 -Vdi D:\ArchieOS-build\vm\ArchieOS.vdi -Out D:\ArchieOS-build [-BootTest]
#
# -BootTest boots it headless, waits for the login prompt on its serial console, takes a
# picture of its screen (the observatory should be on it) and powers it off.
param(
    [Parameter(Mandatory)] [string]$Vdi,
    [Parameter(Mandatory)] [string]$Out,
    [switch]$BootTest,
    # Only the boot test: the machine stays registered, nothing is exported.
    [switch]$NoExport,
    [int]$BootTimeoutSeconds = 600
)
$ErrorActionPreference = 'Stop'
$vbox = Join-Path $env:ProgramFiles 'Oracle\VirtualBox\VBoxManage.exe'
if (-not (Test-Path $vbox)) { throw 'VirtualBox is not installed' }
# VBoxManage writes its banner and progress to stderr, which Windows PowerShell 5.1 turns
# into errors under 'Stop': the exit code decides instead.
function VBox {
    $ErrorActionPreference = 'Continue'
    $output = & $vbox @args 2>&1 | ForEach-Object { "$_" }
    if ($LASTEXITCODE -ne 0) { throw "VBoxManage $($args[0]) failed: $(($output | Where-Object { $_ -match 'error' }) -join ' ')" }
    $output | Where-Object { $_ -and $_ -notmatch '^(Oracle VM VirtualBox|\(C\)|All rights|0%\.\.\.)' }
}
$name = 'ArchieOS-build'
$machines = Join-Path $Out 'machines'
New-Item -ItemType Directory -Force $Out, $machines | Out-Null

# A machine of the same name from an earlier export goes; its disk is not deleted.
if ((& $vbox list vms) -match "^`"$name`"") {
    $ErrorActionPreference = 'Continue'
    & $vbox controlvm $name poweroff 2>$null | Out-Null
    Start-Sleep 2
    # Detached first, so --delete does not take the disk with it (a half-made machine may
    # have no controller at all).
    & $vbox storageattach $name --storagectl SATA --port 0 --device 0 --medium none 2>$null | Out-Null
    $ErrorActionPreference = 'Stop'
    VBox unregistervm $name --delete
}
# A rebuilt disk has a new UUID: VirtualBox's media registry must forget the old one.
$ErrorActionPreference = 'Continue'
& $vbox closemedium disk $Vdi 2>$null | Out-Null
$ErrorActionPreference = 'Stop'
VBox createvm --name $name --ostype ArchLinux_64 --basefolder $machines --register
VBox modifyvm $name --firmware efi --memory 4096 --cpus 4 --rtc-use-utc on `
    --graphicscontroller vmsvga --vram 128 --accelerate-3d on --mouse usbtablet `
    --nic1 nat --nat-localhostreachable1 on --clipboard-mode bidirectional `
    --audio-enabled on --audio-out on --audio-in on --boot1 disk --boot2 none --boot3 none --boot4 none
VBox storagectl $name --name SATA --add sata --controller IntelAhci --portcount 1 --hostiocache on
VBox storageattach $name --storagectl SATA --port 0 --device 0 --type hdd --medium $Vdi

if ($BootTest) {
    $serial = Join-Path $Out 'boot-serial.log'
    Remove-Item $serial -ErrorAction SilentlyContinue
    VBox modifyvm $name --uart1 0x3F8 4 --uart-mode1 file $serial
    VBox startvm $name --type headless
    $deadline = (Get-Date).AddSeconds($BootTimeoutSeconds)
    $booted = $false
    while ((Get-Date) -lt $deadline) {
        Start-Sleep 5
        if ((Test-Path $serial) -and (Select-String -Path $serial -Pattern 'login:' -Quiet)) { $booted = $true; break }
    }
    # The observatory takes a few seconds more than the login prompt.
    Start-Sleep 45
    $shot = Join-Path $Out 'boot-screen.png'
    $ErrorActionPreference = 'Continue'
    & $vbox controlvm $name screenshotpng $shot 2>$null
    & $vbox guestproperty enumerate $name 2>$null | Select-String 'ArchieOS|GuestInfo/OS/Product|Net/0/V4/IP' | ForEach-Object { "  $_" }
    & $vbox controlvm $name poweroff 2>$null | Out-Null
    $ErrorActionPreference = 'Stop'
    Start-Sleep 3
    VBox modifyvm $name --uart1 off
    if (-not $booted) { throw "It did not reach a login prompt in $BootTimeoutSeconds s; see $serial" }
    Write-Host "Booted: the login prompt is on its serial console ($serial); its screen: $shot"
}

if ($NoExport) { Write-Host "Not exported; the machine $name stays registered."; return }
$ova = Join-Path $Out 'ArchieOS.ova'
Remove-Item $ova -ErrorAction SilentlyContinue
VBox export $name --output $ova --ovf20 --manifest --vsys 0 --vmname ArchieOS --product ArchieOS --version 0.1.0 `
    --description 'ArchieOS 0.1: Core, the observatory and its database. The local model is served from Windows by the ArchieOS launcher.'
$hash = (Get-FileHash $ova -Algorithm SHA256).Hash.ToLower()
$sums = Join-Path $Out 'SHA256SUMS'
$lines = @(if (Test-Path $sums) { Get-Content $sums | Where-Object { $_ -notmatch 'ArchieOS\.ova$' } })
Set-Content $sums ($lines + "$hash  ArchieOS.ova") -Encoding ASCII
# The machine was only the vehicle for the export; the disk stays for the next build.
VBox storageattach $name --storagectl SATA --port 0 --device 0 --medium none
VBox unregistervm $name --delete
VBox closemedium disk $Vdi
Write-Host ("ArchieOS.ova: {0:N1} GB, sha256 {1}" -f ((Get-Item $ova).Length / 1GB), $hash)
