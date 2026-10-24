# What this Windows machine is, and how ArchieOS is best installed on it. Dot-sourced by
# the installer and the launcher:
#
#   . .\ArchieOS-Machine.ps1
#   $machine = Get-ArchieMachine
#   $plan = Get-ArchieRecommendation $machine (Read-ArchieCatalog .\local-models.tsv)
#
# Everything is read, nothing is changed. A reading that fails is left empty rather than
# guessed: no GPU found is a CPU-only plan, not a pretend one.

# The GPU runtime ArchieOS serves models with on Windows (for a VirtualBox machine): PrismML's
# llama.cpp, the same release archie-local runs inside WSL, as GitHub lists its assets.
$script:ArchieRuntimeRelease = 'prism-b10787-73bfe6c'
$script:ArchieRuntimes = @{
    'cuda-12.4' = @(
        @{ file = 'llama-prism-b10787-73bfe6c-bin-win-cuda-12.4-x64.zip'; bytes = 314639598; sha256 = '64ca6810aae0e6bbb28d22f888bd009b9bdd0c24c5af8a6a7490b3eaf75ccd57' },
        @{ file = 'cudart-llama-bin-win-cuda-12.4-x64.zip'; bytes = 391443627; sha256 = '8c79a9b226de4b3cacfd1f83d24f962d0773be79f1e7b75c6af4ded7e32ae1d6' })
    'hip-radeon' = @(
        @{ file = 'llama-prism-b10787-73bfe6c-bin-win-hip-radeon-x64.zip'; bytes = 342816702; sha256 = '0e9174d8cb9854c15ebb8d1f6cfe680f39ec488b4fbc673ae07f6a7f08f51d0e' })
    'vulkan' = @(
        @{ file = 'llama-prism-b10787-73bfe6c-bin-win-vulkan-x64.zip'; bytes = 31308770; sha256 = '16d9482224e53d462eb210a766c8d94f0461cdca4d1c7263b2ba7b97092bf739' })
    'cpu' = @(
        @{ file = 'llama-prism-b10787-73bfe6c-bin-win-cpu-x64.zip'; bytes = 19539022; sha256 = 'ab1ea7cfc23fdac244088fd6f1762d0efa24d3d79fe18d4f16c496cc00775728' })
}
# Architectures that release does not know yet (archie-local says the same inside WSL).
$script:ArchieUnknownArches = @('flint', 'deepseek41', 'qwen4exp', 'glm5-next')

function Read-ArchieCatalog([string]$path) {
    Get-Content $path -Encoding UTF8 | Where-Object { $_ -and -not $_.StartsWith('#') } | ForEach-Object {
        $f = $_ -split ' '
        [pscustomobject]@{
            name = $f[0]; file = $f[1]; repo = $f[2]; rev = $f[3]; sha256 = @($f[4] -split ','); bytes = [int64]$f[5]
            port = [int]$f[6]; alias = $f[7]; reasoning = $f[8]; arch = $f[9]; license = $f[10]; tier = [int]$f[11]
            label = ($f[12..($f.Count - 1)] -join ' ')
        }
    }
}

# Video memory as the driver reports it: Win32_VideoController stops at 4 GB, the registry
# has the real size.
function Get-GpuMemoryMb([string]$name) {
    $class = 'HKLM:\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}'
    foreach ($key in Get-ChildItem $class -ErrorAction SilentlyContinue) {
        $p = Get-ItemProperty $key.PSPath -ErrorAction SilentlyContinue
        if ($p.DriverDesc -ne $name) { continue }
        $size = $p.'HardwareInformation.qwMemorySize'
        if ($size) { return [int]([int64]$size / 1MB) }
        $size = $p.'HardwareInformation.MemorySize'
        if ($size -is [byte[]]) { return [int]([BitConverter]::ToUInt32($size, 0) / 1MB) }
        if ($size) { return [int]([int64]$size / 1MB) }
    }
    return 0
}

function Get-ArchieMachine {
    $os = Get-CimInstance Win32_OperatingSystem
    $cs = Get-CimInstance Win32_ComputerSystem
    $cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
    $gpus = @()
    $smi = Join-Path $env:WINDIR 'System32\nvidia-smi.exe'
    $nvidia = @()
    if (Test-Path $smi) {
        $nvidia = @(& $smi --query-gpu=name,memory.total,driver_version --format=csv,noheader,nounits 2>$null | ForEach-Object {
            $f = $_ -split ',\s*'
            @{ name = $f[0].Trim(); vram = ($f[1] -replace '\D', ''); driver = $f[2].Trim() }
        })
    }
    foreach ($v in Get-CimInstance Win32_VideoController) {
        if ($v.Name -match 'Microsoft Basic|Remote Display|Hyper-V|VirtualBox|Parsec|Virtual') { continue }
        $vendor = if ($v.Name -match 'NVIDIA') { 'nvidia' } elseif ($v.Name -match 'AMD|Radeon') { 'amd' } elseif ($v.Name -match 'Intel') { 'intel' } else { 'other' }
        $smiRow = $nvidia | Where-Object { $_.name -eq $v.Name } | Select-Object -First 1
        $mb = if ($smiRow -and $smiRow.vram) { [int]$smiRow.vram } else { Get-GpuMemoryMb $v.Name }
        $gpus += [pscustomobject]@{
            vendor = $vendor; name = $v.Name; vram_mb = $mb
            driver = if ($smiRow) { $smiRow.driver } else { $v.DriverVersion }
            # A GPU that reports no memory of its own shares the machine's (GB10: RTX Spark).
            unified = ($vendor -eq 'nvidia' -and $smiRow -and -not $smiRow.vram)
        }
    }
    $drives = @(Get-CimInstance Win32_LogicalDisk -Filter 'DriveType=3' | ForEach-Object {
        [pscustomobject]@{ letter = $_.DeviceID.TrimEnd(':'); free_gb = [math]::Round($_.FreeSpace / 1GB, 1); size_gb = [math]::Round($_.Size / 1GB, 1); system = ($_.DeviceID -eq $env:SystemDrive) }
    })
    & wsl.exe --status *> $null
    $wsl = $LASTEXITCODE -eq 0
    $vboxManage = Join-Path $env:ProgramFiles 'Oracle\VirtualBox\VBoxManage.exe'
    $vboxVersion = if (Test-Path $vboxManage) { (& $vboxManage --version 2>$null) -replace 'r\d+$', '' } else { $null }
    [pscustomobject]@{
        windows = $os.Caption; build = [int]$os.BuildNumber
        arch = $env:PROCESSOR_ARCHITECTURE
        cpu = $cpu.Name.Trim(); cores = [int]$cpu.NumberOfCores; threads = [int]$cs.NumberOfLogicalProcessors
        ram_mb = [int]($cs.TotalPhysicalMemory / 1MB)
        gpus = $gpus
        drives = $drives
        wsl = $wsl
        virtualbox = $vboxVersion
        hypervisor = [bool]$cs.HypervisorPresent
        virtualization = [bool]$cpu.VirtualizationFirmwareEnabled -or [bool]$cs.HypervisorPresent
    }
}

# The NVIDIA driver a CUDA 12.4 build needs on Windows.
function Test-DriverAtLeast([string]$driver, [double]$minimum) {
    $n = 0.0
    return [double]::TryParse(($driver -replace '[^\d.]', ''), [Globalization.NumberStyles]::Float, [Globalization.CultureInfo]::InvariantCulture, [ref]$n) -and $n -ge $minimum
}

function Get-ArchieRecommendation($machine, $catalog) {
    $gpu = $machine.gpus | Sort-Object vram_mb -Descending | Select-Object -First 1
    $ramGb = [math]::Floor($machine.ram_mb / 1024)
    $runtime = 'cpu'
    if ($gpu -and $gpu.vendor -eq 'nvidia' -and (Test-DriverAtLeast $gpu.driver 551.61)) { $runtime = 'cuda-12.4' }
    elseif ($gpu -and $gpu.vendor -eq 'amd') { $runtime = 'hip-radeon' }
    elseif ($gpu -and $gpu.vendor -in 'nvidia', 'intel') { $runtime = 'vulkan' }

    # The memory a model may take: the GPU's, less room for Windows and the GUI; a machine
    # whose GPU shares its memory keeps 8 GB for the rest; without a GPU, the small ones.
    $why = @()
    if ($gpu -and $gpu.unified) { $budget = $machine.ram_mb - 8192; $why += "$($gpu.name) comparte {0:N0} GB de memoria con la CPU" -f ($machine.ram_mb / 1024) }
    elseif ($gpu -and $gpu.vram_mb -gt 0 -and $runtime -ne 'cpu') { $budget = $gpu.vram_mb - 1500; $why += "$($gpu.name) con {0:N0} GB" -f ($gpu.vram_mb / 1024) }
    else { $budget = 0; $why += 'sin GPU que el runtime use: un modelo pequeño en la CPU' }
    $usable = @($catalog | Where-Object { $_.arch -notin $script:ArchieUnknownArches })
    $model = if ($budget -gt 0) {
        $usable | Where-Object { $_.bytes / 1MB -le $budget } | Sort-Object tier, bytes -Descending | Select-Object -First 1
    } else {
        $usable | Where-Object { $_.name -eq $(if ($ramGb -ge 12) { '9b' } else { '4b' }) } | Select-Object -First 1
    }

    # WSL gives Linux the GPU directly (CUDA in WSL), so it is first wherever it runs;
    # VirtualBox keeps everything in one machine and borrows the GPU from Windows. The
    # ArchieOS image is built for x64 only: an ARM PC (RTX Spark) has neither yet.
    $wslPossible = $machine.build -ge 19041 -and $machine.arch -eq 'AMD64'
    $vboxPossible = $machine.arch -eq 'AMD64'
    $backend = if ($wslPossible) { 'wsl' } elseif ($vboxPossible) { 'virtualbox' } else { 'none' }
    if ($machine.arch -ne 'AMD64') { $why += 'ArchieOS aún no tiene imagen para ARM: este equipo no puede instalarlo todavía' }

    # The drive with the most room, preferring one other than Windows' own when it has
    # enough for the system and the model: a full system drive froze WSL once.
    $need = 40 + [math]::Ceiling(($model.bytes / 1GB))
    $drive = $machine.drives | Where-Object { -not $_.system -and $_.free_gb -ge $need } | Sort-Object free_gb -Descending | Select-Object -First 1
    if (-not $drive) { $drive = $machine.drives | Sort-Object free_gb -Descending | Select-Object -First 1 }

    $memoryGb = [math]::Max(4, [math]::Min($ramGb - 6, [math]::Floor($ramGb * 0.6)))
    [pscustomobject]@{
        backend = $backend
        wsl_possible = $wslPossible
        virtualbox_possible = $vboxPossible
        runtime = $runtime
        model = $model.name
        model_label = $model.label
        model_bytes = $model.bytes
        why = ($why -join '; ')
        drive = $drive.letter
        drive_free_gb = $drive.free_gb
        need_gb = $need
        memory_gb = $memoryGb
        cpus = [math]::Max(2, [math]::Floor($machine.threads / 2))
        autostart = $true
        autostart_delay_s = 10
        open_gui_on_start = $true
        voice_on_start = $false
    }
}
