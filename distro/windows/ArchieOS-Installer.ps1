# ArchieOS on Windows: installs it. What the launcher's "Instalar ArchieOS" runs: it reads the
# release manifest beside this file, checks this machine, downloads the release's artifacts
# into the chosen folder, checks them, imports the image into WSL, and starts, checks and
# stops it with the image's own commands (archie-offline). Each step leaves its state in
# <folder>\state\install.json, so a closed launcher or a reboot picks up where it was.
#
#   ArchieOS-Installer.ps1 probe|plan        [-Root DIR]
#   ArchieOS-Installer.ps1 install           -Root DIR [-Distro NAME]
#   ArchieOS-Installer.ps1 verify            -Root DIR [-Deep]      (-Deep also loads IRIS)
#   ArchieOS-Installer.ps1 repair            -Root DIR [-AllowUnregisterBroken]
#   ArchieOS-Installer.ps1 adopt             -Root DIR -Distro NAME (an existing distribution)
#   ArchieOS-Installer.ps1 start|stop|health|status -Root DIR
#   ArchieOS-Installer.ps1 fix-dependency    -Id wsl|wsl-update [-Root DIR]
#
# -OutFile receives the log lines as they come; -ResultFile (else -OutFile, else the output)
# one line of JSON with the result.
# Without a verb (dot-sourced) it only defines its functions: the launcher and the tests.
# Windows PowerShell 5.1: nothing to install. No token is ever written in clear or logged.
param(
    [string]$Verb = '',
    [string]$Root = '',
    [string]$OutFile = '',
    [string]$ResultFile = '',
    [string]$Id = '',
    [string]$Distro = '',
    [string]$ManifestPath = '',
    [switch]$Deep,
    [switch]$AllowUnregisterBroken,
    [switch]$AllowUnsigned,
    [string]$Lang = ''
)

$script:InstallerDir = $PSScriptRoot
$i18nFile = Join-Path $PSScriptRoot 'ArchieOS-i18n.ps1'
if (Test-Path $i18nFile) { . $i18nFile } elseif (-not (Get-Command T -ErrorAction SilentlyContinue)) {
    function T([string]$text) { $text = $text.Replace('\n', "`n"); if ($args.Count) { return ($text -f $args) } return $text }
    function Initialize-Language([string]$requested = '') { }
}
if ($Verb) { Initialize-Language $Lang }
$script:AllowUnsigned = [bool]$AllowUnsigned
# The release keys this installer trusts (ECDSA P-256; scripts/release/Sign-Release.ps1 makes and
# uses them). A manifest not signed by one of them is not read: it names what gets downloaded.
$script:TrustedKeys = @{
    'bd7e301b5e1a3553' = 'RUNTMSAAAABYIHuAIPVqnWy31fxsf/srzjmbnyw4rWz9cQrF9Wklm3T6r6l41xDyDfoR9SjVoM4lusJn7+3WFwaLOsGLJnS3'
}
$script:Utf8 = New-Object System.Text.UTF8Encoding($false)
$script:OutPath = ''
$script:LogPath = ''
$script:GuestUser = 'archie'
$script:GuestState = '/home/archie/.local/state/archie-offline'
$script:InstallStates = @('not_installed', 'checking', 'needs_dependencies', 'downloading', 'verifying', 'installing',
    'needs_reboot', 'installed', 'starting', 'running', 'stopped', 'error', 'repairable')
# From each state, where it may go; 'checking' and 'error' are reachable from any.
$script:Transitions = @{
    not_installed      = @('downloading')
    checking           = @('not_installed', 'needs_dependencies', 'needs_reboot', 'downloading', 'installed', 'stopped', 'running', 'repairable')
    needs_dependencies = @('needs_reboot')
    needs_reboot       = @()
    downloading        = @('verifying')
    verifying          = @('installing')
    installing         = @('installed', 'repairable')
    installed          = @('starting', 'stopped', 'repairable')
    starting           = @('running')
    running            = @('starting', 'stopped')
    stopped            = @('starting', 'repairable')
    error              = @('downloading', 'installing', 'starting', 'stopped', 'repairable')
    repairable         = @('installing', 'installed', 'stopped')
}

# ---- output, errors, secrets -------------------------------------------------------------
# An error carries a code the launcher can act on: [auth], [not_found], [checksum], ...
function New-InstallError([string]$code, [string]$message) {
    $e = New-Object System.Exception("[$code] $message")
    $e.Data['code'] = $code
    return $e
}
function Get-ErrorCode($exception) {
    for ($e = $exception; $e; $e = $e.InnerException) {
        if ($e.Data -and $e.Data['code']) { return $e.Data['code'] }
        if ($e.Message -match '^\[(\w+)\]') { return $Matches[1] }
    }
    return 'unexpected'
}
function Protect-Secret([string]$text) {
    if (-not $text) { return $text }
    $text = $text -replace '(hf_|gh[pousr]_|github_pat_)[A-Za-z0-9_]{6,}', '$1***'
    return ($text -replace '(?i)(authorization:\s*\w+\s+)\S+', '$1***')
}
function Write-Step([string]$text) {
    if (-not $text) { return }
    $line = Protect-Secret $text.TrimEnd()
    if ($script:OutPath) { [System.IO.File]::AppendAllText($script:OutPath, "$line`n", $script:Utf8) } else { Write-Host $line }
    if ($script:LogPath) {
        [System.IO.File]::AppendAllText($script:LogPath, ('[{0}] {1}{2}' -f (Get-Date -Format 's'), $line, "`n"), $script:Utf8)
    }
}
function Set-Field($object, [string]$name, $value) {
    $object | Add-Member -NotePropertyName $name -NotePropertyValue $value -Force
}

# ---- running programs ------------------------------------------------------------------------
# One argument per item, quoted by CommandLineToArgvW's rules: the way wsl.exe and
# powershell.exe split their command line. Paths with spaces, quotes or accents arrive whole,
# and no shell sits in between to read them.
function ConvertTo-ArgumentString([string[]]$argv) {
    $quoted = foreach ($a in $argv) {
        $a = [string]$a
        if ($a -and $a -notmatch '[\s"]') { $a; continue }
        $q = '"'; $slashes = 0
        foreach ($ch in $a.ToCharArray()) {
            if ($ch -eq '\') { $slashes++; continue }
            if ($ch -eq '"') { $q += ('\' * (2 * $slashes + 1)) + '"' } else { $q += ('\' * $slashes) + $ch }
            $slashes = 0
        }
        $q + ('\' * (2 * $slashes)) + '"'
    }
    return ($quoted -join ' ')
}
function Invoke-Native([string]$file, [string[]]$argv, [int]$timeoutSeconds = 120, [string]$stdin = $null) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $file
    $psi.Arguments = ConvertTo-ArgumentString $argv
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.RedirectStandardInput = $null -ne $stdin
    $psi.StandardOutputEncoding = $script:Utf8
    $psi.StandardErrorEncoding = $script:Utf8
    # wsl.exe writes its own messages in UTF-16 unless told otherwise.
    $psi.EnvironmentVariables['WSL_UTF8'] = '1'
    $p = [System.Diagnostics.Process]::Start($psi)
    $outTask = $p.StandardOutput.ReadToEndAsync()
    $errTask = $p.StandardError.ReadToEndAsync()
    if ($null -ne $stdin) { $p.StandardInput.Write($stdin.Replace("`r`n", "`n")); $p.StandardInput.Close() }
    if (-not $p.WaitForExit($timeoutSeconds * 1000)) {
        try { $p.Kill() } catch { }
        return @{ code = -1; out = ''; err = (T 'no response in {0} s' $timeoutSeconds); timeout = $true }
    }
    $p.WaitForExit()
    return @{ code = $p.ExitCode; out = ($outTask.Result -replace "`0", ''); err = ($errTask.Result -replace "`0", ''); timeout = $false }
}
function Invoke-Wsl([string[]]$argv, [int]$timeoutSeconds = 120, [string]$stdin = $null) {
    return Invoke-Native 'wsl.exe' $argv $timeoutSeconds $stdin
}
# A program inside the distribution, run directly (--exec): no shell reads its arguments.
function Invoke-Guest([string]$distro, [string]$user, [string[]]$command, [int]$timeoutSeconds = 120, [string]$stdin = $null) {
    Assert-DistroName $distro
    return Invoke-Wsl (@('-d', $distro, '-u', $user, '--exec') + $command) $timeoutSeconds $stdin
}
function Assert-DistroName([string]$name) {
    if ($name -notmatch '^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$') { throw (New-InstallError 'invalid' (T 'Invalid distribution name: {0}' $name)) }
}

# ---- PlatformProbe ---------------------------------------------------------------------------
function Get-WslDistroList([switch]$Running) {
    $argv = if ($Running) { @('--list', '--running', '--quiet') } else { @('--list', '--quiet') }
    $r = Invoke-Wsl $argv 30
    if ($r.code -ne 0) { return @() }
    return @($r.out -split "`r?`n" | ForEach-Object { $_.Trim() } | Where-Object { $_ })
}
# Where WSL keeps a distribution, from its registry entry, and whether its disk is there.
function Get-WslRegistration([string]$name) {
    $key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Lxss'
    if (-not (Test-Path $key)) { return $null }
    foreach ($k in Get-ChildItem $key) {
        $p = Get-ItemProperty $k.PSPath
        if ($p.DistributionName -ne $name) { continue }
        $base = [string]$p.BasePath -replace '^\\\\\?\\', ''
        $file = if ($p.VhdFileName) { $p.VhdFileName } else { 'ext4.vhdx' }
        $vhdx = Join-Path $base $file
        return [pscustomobject]@{ name = $name; base_path = $base; vhdx = $vhdx; disk_present = (Test-Path -LiteralPath $vhdx); version = $p.Version }
    }
    return $null
}
function Get-WslVersionInfo {
    if (-not (Get-Command wsl.exe -ErrorAction SilentlyContinue)) { return [pscustomobject]@{ present = $false; store = $false; version = $null; wslg = $false } }
    $r = Invoke-Wsl @('--version') 30
    $lines = @($r.out -split "`r?`n" | Where-Object { $_ -match '\d+\.\d+\.\d+' })
    # The WSL of an older Windows, or the stub of one without WSL, has no --version.
    if ($r.code -ne 0 -or $lines.Count -eq 0) {
        return [pscustomobject]@{ present = $true; store = $false; version = $null; wslg = $false; raw = (($r.out + $r.err).Trim()) }
    }
    $wslg = @($lines | Where-Object { $_ -match 'WSLg' }).Count -gt 0
    $kernel = if ($lines.Count -gt 1) { [regex]::Match($lines[1], '\d+\.\d+\.\d+\S*').Value } else { '' }
    return [pscustomobject]@{ present = $true; store = $true; version = [regex]::Match($lines[0], '\d+\.\d+\.\d+(\.\d+)?').Value; kernel = $kernel; wslg = $wslg }
}
function Read-WslConfig {
    $file = Join-Path $env:USERPROFILE '.wslconfig'
    $cfg = @{}
    if (-not (Test-Path -LiteralPath $file)) { return $cfg }
    $section = ''
    foreach ($line in Get-Content -LiteralPath $file) {
        if ($line -match '^\s*\[(.+)\]') { $section = $Matches[1].Trim().ToLower(); continue }
        if ($section -eq 'wsl2' -and $line -match '^\s*([A-Za-z]+)\s*=\s*([^#;]*)') { $cfg[$Matches[1].ToLower()] = $Matches[2].Trim() }
    }
    return $cfg
}
function ConvertFrom-SizeText([string]$text) {
    if ($text -notmatch '^\s*(\d+(?:\.\d+)?)\s*([KMGT]?)B?\s*$') { return $null }
    $units = @{ '' = 1; K = 1KB; M = 1MB; G = 1GB; T = 1TB }
    return [int64]([double]$Matches[1] * $units[$Matches[2].ToUpper()])
}
function Get-FolderVolume([string]$path) {
    $full = [System.IO.Path]::GetFullPath($path)
    $rootPath = [System.IO.Path]::GetPathRoot($full)
    if ($rootPath -like '\\*') { return [pscustomobject]@{ path = $full; root = $rootPath; network = $true } }
    $d = New-Object System.IO.DriveInfo($rootPath)
    if (-not $d.IsReady) { return [pscustomobject]@{ path = $full; root = $rootPath; ready = $false } }
    return [pscustomobject]@{ path = $full; root = $rootPath; ready = $true; network = ($d.DriveType -eq 'Network'); type = [string]$d.DriveType
        format = $d.DriveFormat; free = $d.AvailableFreeSpace; total = $d.TotalSize }
}
function Get-HostProbe {
    if ([Environment]::OSVersion.Platform -ne 'Win32NT') {
        $release = if (Test-Path '/etc/os-release') { (Get-Content '/etc/os-release' | Where-Object { $_ -like 'PRETTY_NAME=*' }) -replace '^PRETTY_NAME=|"', '' } else { 'Linux' }
        return [pscustomobject]@{ platform = 'linux'; os = $release; arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString(); supported = $false }
    }
    $os = Get-CimInstance Win32_OperatingSystem
    $cs = Get-CimInstance Win32_ComputerSystem
    $cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
    $arch = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
    $admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    $rebootPending = (Test-Path 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\RebootPending') -or
        (Test-Path 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsUpdate\Auto Update\RebootRequired')
    $netfx = (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\NET Framework Setup\NDP\v4\Full' -ErrorAction SilentlyContinue).Release
    $gpus = @(Get-CimInstance Win32_VideoController | ForEach-Object {
        [pscustomobject]@{ name = $_.Name; driver = $_.DriverVersion; nvidia = [bool]($_.Name -match 'NVIDIA') } })
    $wslcfg = Read-WslConfig
    $ram = [int64]$cs.TotalPhysicalMemory
    # Without a memory= line WSL 2 takes half of the machine's memory.
    $wslMemory = if ($wslcfg['memory']) { ConvertFrom-SizeText $wslcfg['memory'] } else { [int64]($ram / 2) }
    return [pscustomobject]@{
        platform = 'windows'; supported = $true
        os = $os.Caption; build = [int]$os.BuildNumber; arch = $arch; admin = $admin
        last_boot = $os.LastBootUpTime.ToUniversalTime().ToString('o')
        virtualization = [pscustomobject]@{ hypervisor_present = [bool]$cs.HypervisorPresent; firmware_enabled = [bool]$cpu.VirtualizationFirmwareEnabled }
        memory_bytes = $ram; wsl_memory_bytes = $wslMemory; networking_mode = $wslcfg['networkingmode']
        reboot_pending = $rebootPending; netfx_release = $netfx; gpus = $gpus
        wsl = Get-WslVersionInfo
    }
}

# ---- ReleaseResolver / ManifestVerifier -----------------------------------------------------------
function Get-DefaultManifestPath { return (Join-Path $script:InstallerDir 'archieos-manifest.json') }
# The manifest pins each artifact: an HTTPS source at a fixed revision, its size and its
# SHA-256. Without a published digest nothing is installed: an unchecked image is not one.
# The manifest's detached signature: <manifest>.sig names the key and signs the manifest's exact
# bytes. Without a valid one from a trusted key nothing is read, unless -AllowUnsigned (development).
function Test-ManifestSignature([string]$path) {
    $sigPath = "$path.sig"
    if (-not (Test-Path -LiteralPath $sigPath)) { throw (New-InstallError 'signature' (T 'The release manifest has no signature (archieos-manifest.json.sig).')) }
    try { $s = Get-Content -LiteralPath $sigPath -Raw -Encoding UTF8 | ConvertFrom-Json } catch { throw (New-InstallError 'signature' (T 'The signature file is not valid JSON.')) }
    if ($s.alg -ne 'ecdsa-p256-sha256') { throw (New-InstallError 'signature' (T 'Unknown signature algorithm: {0}' $($s.alg))) }
    $blob = $script:TrustedKeys[[string]$s.key_id]
    if (-not $blob) { throw (New-InstallError 'signature' (T 'The manifest is signed by an unknown key ({0}). Download a newer installer.' $($s.key_id))) }
    try {
        $key = [Security.Cryptography.CngKey]::Import([Convert]::FromBase64String($blob), [Security.Cryptography.CngKeyBlobFormat]::EccPublicBlob)
        $ec = New-Object Security.Cryptography.ECDsaCng $key
        $ec.HashAlgorithm = [Security.Cryptography.CngAlgorithm]::Sha256
        $ok = $ec.VerifyData([IO.File]::ReadAllBytes($path), [Convert]::FromBase64String([string]$s.signature))
    } catch { $ok = $false }
    if (-not $ok) { throw (New-InstallError 'signature' (T 'The release manifest signature does not match. It was changed or is not from ArchieOS; nothing was installed.')) }
    return [string]$s.key_id
}
function Read-ReleaseManifest([string]$path = '') {
    if (-not $path) { $path = Get-DefaultManifestPath }
    if (Test-Path -LiteralPath $path) {
        if ($script:AllowUnsigned) { Write-Step (T 'WARNING: signature not checked (-AllowUnsigned).') }
        else { $keyId = Test-ManifestSignature $path; Write-Step (T 'Manifest signed by key {0}: valid.' $keyId) }
    }
    if (-not (Test-Path -LiteralPath $path)) { throw (New-InstallError 'manifest' (T 'Cannot find the manifest {0}' $path)) }
    try { $m = Get-Content -LiteralPath $path -Raw -Encoding UTF8 | ConvertFrom-Json } catch { throw (New-InstallError 'manifest' (T 'The manifest is not valid JSON: {0}' $($_.Exception.Message))) }
    if ($m.schema_version -ne 1) { throw (New-InstallError 'manifest' (T 'Unsupported manifest version: {0}' $($m.schema_version))) }
    if (-not $m.release -or $m.release -match 'latest') { throw (New-InstallError 'manifest' (T 'The manifest must pin a version, not "latest".')) }
    $target = $m.targets.'windows-x86_64-wsl2'
    if (-not $target) { throw (New-InstallError 'manifest' (T 'The manifest has no windows-x86_64-wsl2 target.')) }
    Assert-DistroName $target.distro_name
    if ($target.runtime -ne 'archie-offline') { throw (New-InstallError 'manifest' (T 'Unknown startup mechanism: {0}' $($target.runtime))) }
    $ids = @{}
    foreach ($a in @($target.artifacts)) {
        if (-not $a.id -or $ids.ContainsKey($a.id)) { throw (New-InstallError 'manifest' (T 'Every artifact needs a unique id.')) }
        $ids[$a.id] = $a
        if ([string]$a.sha256 -notmatch '^[0-9a-f]{64}$') { throw (New-InstallError 'manifest' (T '{0}: no published SHA-256; nothing is installed that cannot be checked.' $($a.id))) }
        if (-not ([int64]$a.size_bytes -gt 0)) { throw (New-InstallError 'manifest' (T '{0}: the size is missing.' $($a.id))) }
        if ($a.source) {
            $uri = $null
            if (-not [Uri]::TryCreate([string]$a.source, [UriKind]::Absolute, [ref]$uri) -or $uri.Scheme -ne 'https') { throw (New-InstallError 'manifest' (T '{0}: the source must be an HTTPS URL.' $($a.id))) }
            if ($uri.AbsolutePath -match '/latest(/|$)') { throw (New-InstallError 'manifest' (T '{0}: the source cannot point to "latest".' $($a.id))) }
            if ($a.file -notmatch '^[^\\/:*?"<>|]+$') { throw (New-InstallError 'manifest' (T '{0}: invalid file name.' $($a.id))) }
            if ($a.auth -and $a.auth -notin @('huggingface', 'github')) { throw (New-InstallError 'manifest' (T '{0}: unknown authentication {1}.' $($a.id) $($a.auth))) }
            if ($a.format -ne 'wsl-tar-gzip') { throw (New-InstallError 'manifest' (T '{0}: unsupported format {1}.' $($a.id) $($a.format))) }
        } elseif (-not $a.embedded_in) { throw (New-InstallError 'manifest' (T '{0}: it has neither a source nor an image that contains it.' $($a.id))) }
    }
    foreach ($a in @($target.artifacts)) {
        if ($a.embedded_in -and -not $ids.ContainsKey($a.embedded_in)) { throw (New-InstallError 'manifest' (T '{0}: {1} is not in the manifest.' $($a.id) $($a.embedded_in))) }
    }
    if (@($target.artifacts | Where-Object { $_.source -and $_.required }).Count -ne 1) { throw (New-InstallError 'manifest' (T 'The target needs exactly one required image to download.')) }
    return $m
}
function Get-Target($manifest) { return $manifest.targets.'windows-x86_64-wsl2' }
function Get-ImageArtifact($manifest) { return @((Get-Target $manifest).artifacts | Where-Object { $_.source -and $_.required })[0] }

# ---- credentials ---------------------------------------------------------------------------------
# A token is kept encrypted for this Windows user with DPAPI (ConvertFrom-SecureString), in
# %APPDATA%\ArchieOS\credentials. It is read only to make the request, never logged.
function Get-CredentialFile([string]$provider) {
    if ($provider -notin @('huggingface', 'github')) { throw (New-InstallError 'invalid' "Proveedor desconocido: $provider") }
    return (Join-Path $env:APPDATA "ArchieOS\credentials\$provider.dpapi")
}
function Save-ProviderToken([string]$provider, [System.Security.SecureString]$token) {
    $file = Get-CredentialFile $provider
    New-Item -ItemType Directory -Force (Split-Path $file) | Out-Null
    [System.IO.File]::WriteAllText($file, (ConvertFrom-SecureString $token), $script:Utf8)
}
function Test-ProviderToken([string]$provider) { return (Test-Path -LiteralPath (Get-CredentialFile $provider)) }
function Remove-ProviderToken([string]$provider) { Remove-Item -LiteralPath (Get-CredentialFile $provider) -ErrorAction SilentlyContinue }
function Get-ProviderToken([string]$provider) {
    $file = Get-CredentialFile $provider
    if (Test-Path -LiteralPath $file) {
        $secure = ConvertTo-SecureString ([System.IO.File]::ReadAllText($file).Trim())
        $bstr = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure)
        try { return [Runtime.InteropServices.Marshal]::PtrToStringBSTR($bstr) } finally { [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($bstr) }
    }
    # For GitHub, the sign-in GitHub CLI already holds, if any.
    if ($provider -eq 'github' -and (Get-Command gh.exe -ErrorAction SilentlyContinue)) {
        $r = Invoke-Native 'gh.exe' @('auth', 'token') 20
        if ($r.code -eq 0 -and $r.out.Trim()) { return $r.out.Trim() }
    }
    return $null
}
function Get-AuthHost([string]$provider) {
    switch ($provider) { 'huggingface' { 'huggingface.co' } 'github' { 'api.github.com' } default { '' } }
}

# ---- InstallStateStore -------------------------------------------------------------------------------
function Get-StateFile([string]$root) { return (Join-Path $root 'state\install.json') }
function Read-InstallState([string]$root) {
    $file = Get-StateFile $root
    if ($root -and (Test-Path -LiteralPath $file)) {
        try { return (Get-Content -LiteralPath $file -Raw -Encoding UTF8 | ConvertFrom-Json) } catch { }
    }
    return [pscustomobject]@{ schema = 1; state = 'not_installed' }
}
# Written beside itself and then swapped in: a crash leaves the old state or the new, whole.
function Save-InstallState([string]$root, $st) {
    $file = Get-StateFile $root
    New-Item -ItemType Directory -Force (Split-Path $file) | Out-Null
    Set-Field $st 'updated_at' ((Get-Date).ToUniversalTime().ToString('o'))
    $tmp = "$file.tmp"
    [System.IO.File]::WriteAllText($tmp, ($st | ConvertTo-Json -Depth 8), $script:Utf8)
    if (Test-Path -LiteralPath $file) { [System.IO.File]::Replace($tmp, $file, $null) } else { [System.IO.File]::Move($tmp, $file) }
}
function Test-Transition([string]$from, [string]$to) {
    if ($to -notin $script:InstallStates) { return $false }
    if ($to -in @('checking', 'error') -or $from -eq $to) { return $true }
    return ($script:Transitions[$from] -contains $to)
}
function Set-InstallState([string]$root, $st, [string]$state, [hashtable]$fields = @{}) {
    if (-not (Test-Transition $st.state $state)) { throw (New-InstallError 'state' (T 'Invalid state transition: {0} → {1}' $($st.state) $state)) }
    Set-Field $st 'state' $state
    foreach ($k in $fields.Keys) { Set-Field $st $k $fields[$k] }
    Save-InstallState $root $st
    Write-Step "estado: $state"
}

# ---- DependencyManager -----------------------------------------------------------------------------------
function Compare-Version([string]$a, [string]$b) {
    $va = [version](($a -split '\.' | Select-Object -First 4) -join '.')
    $vb = [version](($b -split '\.' | Select-Object -First 4) -join '.')
    return $va.CompareTo($vb)
}
function New-Dependency([string]$id, [string]$name, [string]$status, [string]$detail, [string]$fix = '', [string]$fixId = '', [bool]$elevated = $false, [bool]$reboot = $false) {
    # installed | outdated | missing | unsupported | reboot_required | warning (does not block)
    return [pscustomobject]@{ id = $id; name = $name; status = $status; detail = $detail; fix = $fix; fix_id = $fixId
        elevated = $elevated; reboot = $reboot; blocking = ($status -notin @('installed', 'warning')) }
}
# What this machine has and lacks for the target, each with how it is put right. Nothing is
# changed here; Invoke-DependencyFix does that, after the person says yes.
function Get-DependencyPlan($probe, $manifest, [string]$root = '', [string]$distroName = '', $st = $null) {
    $target = Get-Target $manifest
    $image = Get-ImageArtifact $manifest
    if (-not $distroName) { $distroName = $target.distro_name }
    $plan = New-Object System.Collections.ArrayList
    if ($probe.platform -ne 'windows') {
        [void]$plan.Add((New-Dependency 'platform' (T 'System') 'unsupported' (T '{0}: this launcher installs ArchieOS on Windows with WSL 2. There is no published package for Linux: only archiso ISOs, which are boot media and are not written to disks from here.' $($probe.os))))
        return $plan
    }
    if ($probe.build -ge 22000) { [void]$plan.Add((New-Dependency 'windows' 'Windows' 'installed' (T '{0} (build {1})' $($probe.os) $($probe.build)))) }
    elseif ($probe.build -ge 19044) { [void]$plan.Add((New-Dependency 'windows' 'Windows' 'installed' (T '{0} (build {1}); WSLg comes with the Microsoft Store WSL' $($probe.os) $($probe.build)))) }
    else { [void]$plan.Add((New-Dependency 'windows' 'Windows' 'unsupported' (T 'Build {0}: WSLg needs Windows 11, or Windows 10 21H2 (19044) or later.' $($probe.build)) 'Actualiza Windows.')) }
    if ($probe.arch -eq 'AMD64') { [void]$plan.Add((New-Dependency 'arch' 'Arquitectura' 'installed' 'x86_64')) }
    else { [void]$plan.Add((New-Dependency 'arch' 'Arquitectura' 'unsupported' (T '{0}: the ArchieOS image is x86_64.' $($probe.arch)))) }
    if ($probe.virtualization.hypervisor_present -or $probe.virtualization.firmware_enabled) {
        [void]$plan.Add((New-Dependency 'virtualization' (T 'Virtualization') 'installed' $(if ($probe.virtualization.hypervisor_present) { (T 'Windows hypervisor active') } else { (T 'enabled in the firmware') })))
    } else {
        [void]$plan.Add((New-Dependency 'virtualization' (T 'Virtualization') 'missing' (T 'Virtualization is not enabled on the CPU.') (T 'Enable it (Intel VT-x or AMD-V/SVM) in the BIOS/UEFI; it cannot be done from Windows.')))
    }
    $wsl = $probe.wsl
    $minWsl = [string]$target.min_wsl_version
    $rebootFirst = $st -and $st.state -eq 'needs_reboot'
    if ($rebootFirst) {
        [void]$plan.Add((New-Dependency 'wsl' 'WSL 2' 'reboot_required' (T 'WSL was installed: Windows has to restart before going on.') (T 'Restart Windows and open the launcher again.') '' $false $true))
    } elseif (-not $wsl.present -or -not $wsl.store) {
        [void]$plan.Add((New-Dependency 'wsl' 'WSL 2' 'missing' (T 'WSL is not installed (or it is the old WSL built into Windows).') (T 'wsl --install --no-distribution: installs WSL and the Virtual Machine Platform, with no distribution. It asks for administrator permission and a restart.') 'wsl' $true $true))
    } elseif ((Compare-Version $wsl.version $minWsl) -lt 0) {
        [void]$plan.Add((New-Dependency 'wsl' 'WSL 2' 'outdated' (T 'WSL {0}; the image needs {1} or later.' $($wsl.version) $minWsl) (T 'wsl --update: updates WSL from Microsoft. It asks for administrator permission.') 'wsl-update' $true $false))
    } else {
        [void]$plan.Add((New-Dependency 'wsl' 'WSL 2' 'installed' "WSL $($wsl.version), kernel $($wsl.kernel)"))
    }
    if ($wsl.store -and -not $rebootFirst) {
        if ($wsl.wslg) { [void]$plan.Add((New-Dependency 'wslg' (T 'WSLg (display and sound)') 'installed' (T 'included in this WSL'))) }
        else { [void]$plan.Add((New-Dependency 'wslg' (T 'WSLg (display and sound)') 'missing' (T 'This WSL does not report WSLg; the desktop and the voice need it.') 'wsl --update' 'wsl-update' $true $false)) }
    }
    if ($probe.reboot_pending) {
        [void]$plan.Add((New-Dependency 'reboot' (T 'Pending restart') 'warning' (T 'Windows has a pending restart. If WSL fails to import, restart before trying again.')))
    }
    $min = [int64]$target.min_wsl_memory_bytes
    if ($probe.wsl_memory_bytes -ge $min) { [void]$plan.Add((New-Dependency 'memory' (T 'Memory for WSL') 'installed' (Format-Size $probe.wsl_memory_bytes))) }
    else {
        [void]$plan.Add((New-Dependency 'memory' (T 'Memory for WSL') 'warning' ((T 'WSL will have {0}; the release asks for {1}. It may be slow or fail to load the model.') -f (Format-Size $probe.wsl_memory_bytes), (Format-Size $min)) (T 'If needed, set memory=8GB in the [wsl2] section of %USERPROFILE%\.wslconfig.')))
    }
    $nvidia = @($probe.gpus | Where-Object { $_.nvidia })
    if ($nvidia.Count) { [void]$plan.Add((New-Dependency 'gpu' 'GPU NVIDIA' 'installed' ((T '{0}, driver {1}. The image starts on CPU; the GPU is not used by default.') -f $nvidia[0].name, $nvidia[0].driver))) }
    else { [void]$plan.Add((New-Dependency 'gpu' 'GPU NVIDIA' 'warning' (T 'There is no NVIDIA GPU: ArchieOS runs on CPU, its default mode (slow voice answers).'))) }
    if ($probe.netfx_release -ge 378389) { [void]$plan.Add((New-Dependency 'netfx' (T '.NET Framework (launcher window)') 'installed' (T '4.x, version {0}' $($probe.netfx_release)))) }
    else { [void]$plan.Add((New-Dependency 'netfx' (T '.NET Framework (launcher window)') 'missing' (T 'The launcher window needs .NET Framework 4.5 or later.') (T 'Install it from Windows Update.'))) }
    if ($root) { [void]$plan.Add((Get-SpaceDependency $root $image $target $distroName $st)) }
    if ($wsl.store -and -not $rebootFirst) { [void]$plan.Add((Get-DistroDependency $root $distroName $st)) }
    return $plan
}
function Format-Size([double]$b) { if ($b -ge 1e9) { '{0:N1} GB' -f ($b / 1e9) } else { '{0:N0} MB' -f ($b / 1e6) } }
# Room for what is still to come: the rest of the download, and the distribution's disk unless
# it is already imported.
function Get-RequiredBytes([string]$root, $image, $target, $st = $null) {
    $dest = Join-Path $root "downloads\$($image.file)"
    $have = 0
    if (Test-Path -LiteralPath $dest) { $have = (Get-Item -LiteralPath $dest).Length }
    elseif (Test-Path -LiteralPath "$dest.part") { $have = (Get-Item -LiteralPath "$dest.part").Length }
    $download = [math]::Max([int64]0, [int64]$image.size_bytes - $have)
    $imported = $st -and $st.distro -and $st.state -in @('installed', 'running', 'stopped', 'starting')
    $disk = if ($imported) { 0 } else { [int64]$target.install_bytes_estimate }
    return [pscustomobject]@{ download = $download; install = $disk; total = $download + $disk }
}
function Get-SpaceDependency([string]$root, $image, $target, [string]$distroName, $st) {
    $vol = Get-FolderVolume $root
    if ($vol.network) { return (New-Dependency 'space' (T 'Folder') 'unsupported' (T '{0} is on a network drive: WSL does not keep its disk there.' $($vol.path)) (T 'Choose a folder on a local disk.')) }
    if (-not $vol.ready) { return (New-Dependency 'space' (T 'Folder') 'missing' (T 'Drive {0} is not available.' $($vol.root)) (T 'Connect the disk or choose another folder.')) }
    if ($vol.format -notin @('NTFS', 'ReFS')) { return (New-Dependency 'space' (T 'Folder') 'unsupported' (T '{0} is {1}: the WSL virtual disk needs NTFS or ReFS (FAT32 and exFAT will not do).' $($vol.root) $($vol.format)) (T 'Choose another drive.')) }
    if ((Test-Path -LiteralPath $root) -and ((Get-Item -LiteralPath $root).Attributes -band [IO.FileAttributes]::Compressed)) {
        return (New-Dependency 'space' (T 'Folder') 'unsupported' (T '{0} is compressed by NTFS: WSL 2 does not mount a compressed virtual disk.' $root) (T 'Remove the compression from the folder or choose another.'))
    }
    $need = Get-RequiredBytes $root $image $target $st
    $where = "$($vol.root) ($($vol.type))"
    if ($vol.free -lt $need.total) {
        return (New-Dependency 'space' (T 'Disk space') 'missing' ((T '{0} are needed (download {1} + distribution {2}) and {3} has {4} left.') -f (Format-Size $need.total), (Format-Size $need.download), (Format-Size $need.install), $where, (Format-Size $vol.free)) (T 'Free up space or choose another drive.'))
    }
    return (New-Dependency 'space' (T 'Disk space') 'installed' ((T '{0} free on {1}; {2} needed') -f (Format-Size $vol.free), $where, (Format-Size $need.total)))
}
# A distribution with the same name: reused only if it is this installation's, never replaced.
function Get-DistroDependency([string]$root, [string]$name, $st) {
    $reg = Get-WslRegistration $name
    if (-not $reg) { return (New-Dependency 'distro' (T 'Distribution {0}' $name) 'installed' (T 'free: it will be created on install')) }
    $ours = $st -and $st.distro -eq $name -and $st.location -and ([IO.Path]::GetFullPath($st.location) -eq [IO.Path]::GetFullPath($reg.base_path))
    if (-not $reg.disk_present) {
        return (New-Dependency 'distro' (T 'Distribution {0}' $name) 'missing' (T 'It is registered in WSL, but its disk {0} does not exist.' $($reg.vhdx)) (T 'Repair: remove that broken registration (it has no data to lose) and import again; or use another name.'))
    }
    if ($ours) { return (New-Dependency 'distro' (T 'Distribution {0}' $name) 'installed' (T 'installed by this launcher in {0}' $($reg.base_path))) }
    return (New-Dependency 'distro' (T 'Distribution {0}' $name) 'missing' (T 'A distribution {0} already exists in {1} that this folder did not install.' $name $($reg.base_path)) (T 'Reuse it (it is checked first) or install under another name. It is not overwritten or deleted.'))
}
# The fixes there are: Microsoft's own WSL commands, elevated with the person's consent
# (Windows asks). Anything else (BIOS, Windows Update, disks) the plan explains.
function Invoke-DependencyFix([string]$id, [string]$root = '') {
    $argv = switch ($id) { 'wsl' { @('--install', '--no-distribution') } 'wsl-update' { @('--update') } default { throw (New-InstallError 'invalid' (T 'There is no automatic fix for {0}' $id)) } }
    Write-Step "wsl.exe $($argv -join ' ') (como administrador)"
    try { $p = Start-Process wsl.exe -ArgumentList $argv -Verb RunAs -Wait -PassThru -WindowStyle Normal }
    catch { throw (New-InstallError 'elevation' (T 'Administrator permission was not given: nothing was changed.')) }
    if ($p.ExitCode -ne 0) { throw (New-InstallError 'dependency' (T 'wsl.exe {0} exited with code {1}.' $($argv -join ' ') $($p.ExitCode))) }
    if ($id -eq 'wsl' -and $root) {
        $st = Read-InstallState $root
        Set-InstallState $root $st 'checking'
        Set-InstallState $root $st 'needs_reboot' @{ reboot_requested_at = (Get-Date).ToUniversalTime().ToString('o') }
    }
    return @{ fixed = $id; reboot = ($id -eq 'wsl') }
}
# After the reboot WSL asked for, the state moves on by itself.
function Update-RebootState([string]$root, $st, $probe) {
    if ($st.state -eq 'needs_reboot' -and $st.reboot_requested_at -and $probe.last_boot -and ([datetime]$probe.last_boot -gt [datetime]$st.reboot_requested_at)) {
        Set-InstallState $root $st 'checking'
        Set-InstallState $root $st 'not_installed'
    }
}

# ---- DownloadManager ---------------------------------------------------------------------------------------
function Write-Progress-File([string]$root, [hashtable]$p) {
    $file = Join-Path $root 'state\progress.json'
    $p['updated_at'] = (Get-Date).ToUniversalTime().ToString('o')
    $tmp = "$file.tmp"
    [System.IO.File]::WriteAllText($tmp, (ConvertTo-Json $p -Compress), $script:Utf8)
    if (Test-Path -LiteralPath $file) { [System.IO.File]::Replace($tmp, $file, $null) } else { [System.IO.File]::Move($tmp, $file) }
}
function Test-CancelRequested([string]$root) { return (Test-Path -LiteralPath (Join-Path $root 'state\cancel')) }
# One HTTP request, following redirects by hand: the token goes to the host it belongs to
# (huggingface.co, api.github.com) and never to the storage it redirects to.
function Open-ArtifactResponse([string]$url, [string]$auth, [int64]$from) {
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    $token = if ($auth) { Get-ProviderToken $auth } else { $null }
    if ($auth -and -not $token) { throw (New-InstallError 'auth' (T 'The download needs access to {0}: save a token in Install → Access.' $(Get-AuthHost $auth))) }
    $authHost = Get-AuthHost $auth
    for ($hop = 0; $hop -lt 8; $hop++) {
        $uri = [Uri]$url
        $loopback = $uri.IsLoopback
        if ($uri.Scheme -ne 'https' -and -not $loopback) { throw (New-InstallError 'insecure' (T 'Redirect to a URL that is not HTTPS: {0}' $($uri.Host))) }
        $req = [Net.HttpWebRequest]::Create($uri)
        $req.AllowAutoRedirect = $false
        $req.Timeout = 30000
        $req.ReadWriteTimeout = 60000
        $req.UserAgent = 'ArchieOS-Launcher'
        if ($from -gt 0) { $req.AddRange($from) }
        if ($token -and $uri.Host -eq $authHost) {
            $req.Headers['Authorization'] = "Bearer $token"
            if ($auth -eq 'github') { $req.Accept = 'application/octet-stream' }
        }
        $resp = $null
        try { $resp = $req.GetResponse() }
        catch {
            $we = $_.Exception
            while ($we -and -not ($we -is [Net.WebException])) { $we = $we.InnerException }
            if (-not $we -or -not $we.Response) { throw (New-InstallError 'network' "$($uri.Host): $($_.Exception.Message)") }
            $resp = $we.Response
        }
        $code = [int]$resp.StatusCode
        if ($code -in @(301, 302, 303, 307, 308)) {
            $next = $resp.Headers['Location']; $resp.Close()
            $url = (New-Object Uri($uri, $next)).AbsoluteUri
            continue
        }
        if ($code -in @(401, 403)) { $resp.Close(); throw (New-InstallError 'auth' (T '{0} answered {1}: the token has no access to this file (or it expired).' $($uri.Host) $code)) }
        if ($code -eq 404) { $resp.Close(); throw (New-InstallError 'not_found' (T '{0} answered 404: the release file does not exist at that address.' $($uri.Host))) }
        if ($code -eq 416) { $resp.Close(); return $null }
        if ($code -ge 400) { $resp.Close(); throw (New-InstallError 'network' (T '{0} answered {1}.' $($uri.Host) $code)) }
        return $resp
    }
    throw (New-InstallError 'network' 'Demasiadas redirecciones.')
}
function Get-FileSha256([string]$path) {
    return (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLower()
}
# Streams the artifact to <file>.part, hashing it as it arrives, resuming with a Range request
# after a cut, and renames it into place only once its size and SHA-256 are the manifest's.
function Invoke-ArtifactDownload($artifact, [string]$dir, [string]$root, [int]$retries = 4) {
    New-Item -ItemType Directory -Force $dir | Out-Null
    $dest = Join-Path $dir $artifact.file
    $part = "$dest.part"
    $total = [int64]$artifact.size_bytes
    if (Test-Path -LiteralPath $dest) {
        Write-Step (T '{0} is already downloaded: checking its SHA-256' $($artifact.file))
        if ((Get-Item -LiteralPath $dest).Length -eq $total -and (Get-FileSha256 $dest) -eq $artifact.sha256) { return $dest }
        Write-Step (T 'It does not match the release: downloading again.')
        Remove-Item -LiteralPath $dest
    }
    $sha = [Security.Cryptography.SHA256]::Create()
    $have = [int64]0
    $buffer = New-Object byte[] (1MB)
    if (Test-Path -LiteralPath $part) {
        $have = (Get-Item -LiteralPath $part).Length
        if ($have -gt $total) { Remove-Item -LiteralPath $part; $have = 0 }
        else {
            Write-Step ((T 'Resuming the download at {0} of {1}') -f (Format-Size $have), (Format-Size $total))
            $fs = [IO.File]::OpenRead($part)
            try { while (($n = $fs.Read($buffer, 0, $buffer.Length)) -gt 0) { [void]$sha.TransformBlock($buffer, 0, $n, $null, 0) } } finally { $fs.Dispose() }
        }
    }
    $vol = Get-FolderVolume $dir
    if ($vol.free -lt ($total - $have)) { throw (New-InstallError 'space' ((T '{0} more free are needed on {1} for the download.') -f (Format-Size ($total - $have)), $vol.root)) }
    $attempt = 0
    while ($have -lt $total) {
        if (Test-CancelRequested $root) { throw (New-InstallError 'cancelled' (T 'Download cancelled; what was downloaded is kept and Retry continues.')) }
        try {
            $resp = Open-ArtifactResponse $artifact.source $artifact.auth $have
            if ($null -eq $resp) { break }
            if ($have -gt 0 -and [int]$resp.StatusCode -eq 200) {
                Write-Step (T 'The server does not resume downloads: starting from zero.')
                $sha.Dispose(); $sha = [Security.Cryptography.SHA256]::Create(); $have = 0
                Remove-Item -LiteralPath $part -ErrorAction SilentlyContinue
            }
            $in = $resp.GetResponseStream()
            $out = New-Object IO.FileStream($part, [IO.FileMode]::Append, [IO.FileAccess]::Write, [IO.FileShare]::Read)
            try {
                $clock = [Diagnostics.Stopwatch]::StartNew(); $mark = $have; $last = 0
                while (($n = $in.Read($buffer, 0, $buffer.Length)) -gt 0) {
                    $out.Write($buffer, 0, $n)
                    [void]$sha.TransformBlock($buffer, 0, $n, $null, 0)
                    $have += $n
                    if ($have -gt $total) { throw (New-InstallError 'checksum' (T 'The server sends more bytes than the release has.')) }
                    if ($clock.ElapsedMilliseconds - $last -ge 500) {
                        $last = $clock.ElapsedMilliseconds
                        $speed = [int64](($have - $mark) / [math]::Max(0.001, $clock.Elapsed.TotalSeconds))
                        Write-Progress-File $root @{ phase = 'downloading'; artifact = $artifact.id; file = $artifact.file; received = $have; total = $total; speed_bps = $speed
                            eta_s = $(if ($speed -gt 0) { [int](($total - $have) / $speed) } else { $null }) }
                        if (Test-CancelRequested $root) { throw (New-InstallError 'cancelled' (T 'Download cancelled; what was downloaded is kept and Retry continues.')) }
                    }
                }
            } finally { $out.Dispose(); $in.Dispose(); $resp.Close() }
            $attempt = 0
        } catch {
            # A cut connection is a network error, or an IOException from the stream that
            # PowerShell hands over wrapped in the call that raised it.
            $code = Get-ErrorCode $_.Exception
            $io = $false
            for ($e = $_.Exception; $e; $e = $e.InnerException) { if ($e -is [IO.IOException] -or $e -is [Net.WebException]) { $io = $true } }
            if ($code -ne 'network' -and -not ($code -eq 'unexpected' -and $io)) { throw }
            $attempt++
            if ($attempt -gt $retries) { throw (New-InstallError 'network' (T 'The download was cut {0} times: {1}. Retry continues where it left off.' $attempt $($_.Exception.Message))) }
            Write-Step (T 'The connection dropped ({0}); retry {1} of {2}' $($_.Exception.Message) $attempt $retries)
            Start-Sleep -Seconds ([math]::Min(30, [math]::Pow(2, $attempt)))
        }
    }
    [void]$sha.TransformFinalBlock($buffer, 0, 0)
    $digest = -join ($sha.Hash | ForEach-Object { $_.ToString('x2') })
    $sha.Dispose()
    if ($have -ne $total -or $digest -ne $artifact.sha256) {
        Remove-Item -LiteralPath $part -ErrorAction SilentlyContinue
        throw (New-InstallError 'checksum' (T '{0} does not match the release (SHA-256 {1}). What was downloaded was deleted; Retry downloads it again.' $($artifact.file) $digest))
    }
    Move-Item -LiteralPath $part -Destination $dest
    Write-Step "$($artifact.file): SHA-256 correcto"
    return $dest
}
# What the image is, from its bytes and not its name: a gzip stream holding a tar archive is
# what wsl --import takes. Returns $null when it is, else why not.
function Test-ArtifactFormat([string]$path, [string]$format) {
    if ($format -ne 'wsl-tar-gzip') { return (T 'format {0} not supported' $format) }
    $fs = [IO.File]::OpenRead($path)
    try {
        $magic = New-Object byte[] 2
        if ($fs.Read($magic, 0, 2) -ne 2 -or $magic[0] -ne 0x1f -or $magic[1] -ne 0x8b) { return (T 'not a gzip file') }
        $fs.Position = 0
        $gz = New-Object IO.Compression.GZipStream($fs, [IO.Compression.CompressionMode]::Decompress, $true)
        $header = New-Object byte[] 512
        $n = 0
        while ($n -lt 512) { $r = $gz.Read($header, $n, 512 - $n); if ($r -le 0) { break }; $n += $r }
        if ($n -lt 512) { return (T 'the gzip does not contain a complete tar') }
        if ([Text.Encoding]::ASCII.GetString($header, 257, 5) -ne 'ustar') { return (T 'the gzip does not contain a tar file') }
        return $null
    } finally { $fs.Dispose() }
}

# ---- RuntimeController -------------------------------------------------------------------------------------
# Asked inside the distribution, in one go: the processes archie-offline started and owns
# (its pid and start time, as it records them), Core's status, the display's API and Bonsai.
$script:HealthProbe = @'
import json, subprocess, urllib.request, pathlib
state = pathlib.Path('/home/archie/.local/state/archie-offline')
result = {}
def owned(name):
    try:
        record = json.loads((state / (name + '.json')).read_text())
        fields = pathlib.Path('/proc', str(record['pid']), 'stat').read_text().rsplit(')', 1)[1].split()
        return fields[0] != 'Z' and fields[19] == record['start_ticks']
    except Exception:
        return False
for name in ['bonsai', 'core', 'displayd', 'display', 'voice']:
    result[name] = owned(name)
try:
    result['core_status'] = subprocess.run(['archie', '--root', str(state / 'node'), 'status'], capture_output=True, timeout=5).returncode == 0
except Exception:
    result['core_status'] = False
def get(url):
    try:
        with urllib.request.urlopen(url, timeout=3) as response:
            return json.load(response)
    except Exception:
        return None
lumen = get('http://127.0.0.1:7666/api/orb/lumen')
result['display_api'] = bool(lumen and lumen.get('root_id'))
health = get('http://127.0.0.1:8081/health')
result['bonsai_health'] = bool(health and health.get('status') == 'ok')
print(json.dumps(result))
'@
function Get-LastJsonLine([string]$text) {
    $line = @($text -split "`r?`n" | Where-Object { $_.Trim().StartsWith('{') } | Select-Object -Last 1)
    if (-not $line) { return $null }
    try { return ($line[0] | ConvertFrom-Json) } catch { return $null }
}
function Test-ArchieHealth([string]$distro) {
    $running = (Get-WslDistroList -Running) -contains $distro
    $h = [pscustomobject]@{ wsl = $running; ok = $false; components = $null; detail = '' }
    if (-not $running) { $h.detail = (T 'the distribution is not running'); return $h }
    $r = Invoke-Guest $distro $script:GuestUser @('/usr/bin/python3', '-') 40 $script:HealthProbe
    $c = Get-LastJsonLine $r.out
    if (-not $c) { $h.detail = (T 'could not read the state: {0}' $($r.err.Trim())); return $h }
    $h.components = $c
    $h.ok = $c.core -and $c.core_status -and $c.displayd -and $c.display -and $c.display_api -and $c.bonsai -and $c.bonsai_health
    $missing = @('core', 'core_status', 'displayd', 'display', 'display_api', 'bonsai', 'bonsai_health' | Where-Object { -not $c.$_ })
    $h.detail = if ($h.ok) { 'Core, displayd, escritorio y Bonsai responden' } else { 'no responden: ' + ($missing -join ', ') }
    return $h
}
# WSL stops a distribution once no wsl.exe is attached to it, background services and all; a
# hidden one, owned by the launcher, keeps ArchieOS up until Detener.
function Start-KeepAlive([string]$root, $st) {
    if ($st.keepalive_pid) {
        $p = Get-Process -Id $st.keepalive_pid -ErrorAction SilentlyContinue
        if ($p -and $p.ProcessName -eq 'wsl' -and $p.StartTime.ToUniversalTime().ToString('o') -eq $st.keepalive_started) { return }
    }
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = 'wsl.exe'
    $psi.Arguments = ConvertTo-ArgumentString @('-d', $st.distro, '-u', $script:GuestUser, '--exec', '/usr/bin/sleep', 'infinity')
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    $p = [System.Diagnostics.Process]::Start($psi)
    Set-Field $st 'keepalive_pid' $p.Id
    Set-Field $st 'keepalive_started' $p.StartTime.ToUniversalTime().ToString('o')
    Save-InstallState $root $st
}
function Stop-KeepAlive([string]$root, $st) {
    if (-not $st.keepalive_pid) { return }
    $p = Get-Process -Id $st.keepalive_pid -ErrorAction SilentlyContinue
    # Only the process this installer started: same pid and the same start time.
    if ($p -and $p.ProcessName -eq 'wsl' -and $p.StartTime.ToUniversalTime().ToString('o') -eq $st.keepalive_started) { $p.Kill() }
    Set-Field $st 'keepalive_pid' $null
    Set-Field $st 'keepalive_started' $null
}
function Get-InstalledState([string]$root) {
    $st = Read-InstallState $root
    if (-not $st.distro -or $st.state -in @('not_installed', 'checking', 'needs_dependencies', 'downloading', 'verifying', 'installing', 'needs_reboot')) {
        throw (New-InstallError 'not_installed' (T 'ArchieOS is not installed in this folder.'))
    }
    $reg = Get-WslRegistration $st.distro
    if (-not $reg -or -not $reg.disk_present) {
        Set-InstallState $root $st 'checking'
        Set-InstallState $root $st 'repairable' @{ error = (T 'The distribution {0} is no longer registered in WSL or its disk is missing.' $($st.distro)) }
        throw (New-InstallError 'repairable' (T 'The distribution {0} is gone (or its disk is missing): Verify/Repair re-imports it.' $($st.distro)))
    }
    return $st
}
function Start-ArchieRuntime([string]$root) {
    $st = Get-InstalledState $root
    $h = Test-ArchieHealth $st.distro
    if ($h.ok) {
        # Already up: a second start would not start a second ArchieOS, only report this one.
        Start-KeepAlive $root $st
        Set-InstallState $root $st 'running' @{ health = $h; error = $null }
        return @{ state = 'running'; health = $h; already = $true }
    }
    Set-InstallState $root $st 'starting' @{ error = $null }
    Start-KeepAlive $root $st
    Write-Step (T 'archie-offline --background in {0} (the first time it prepares the database)' $($st.distro))
    $r = Invoke-Guest $st.distro $script:GuestUser @('/usr/bin/archie-offline', '--background') 900
    ($r.out + "`n" + $r.err) -split "`r?`n" | ForEach-Object { Write-Step $_ }
    if ($r.code -ne 0) {
        $why = (($r.err + $r.out).Trim() -split "`r?`n" | Select-Object -Last 1)
        Set-InstallState $root $st 'error' @{ error = (T 'It did not start: {0}' $why); error_code = 'start' }
        throw (New-InstallError 'start' (T 'ArchieOS did not start: {0}' $why))
    }
    # Running is what answers, not what was launched.
    $deadline = (Get-Date).AddSeconds(120)
    do {
        $h = Test-ArchieHealth $st.distro
        if ($h.ok) { break }
        Start-Sleep -Seconds 3
    } while ((Get-Date) -lt $deadline)
    if (-not $h.ok) {
        Set-InstallState $root $st 'error' @{ error = (T 'It started, but {0}' $($h.detail)); error_code = 'health'; health = $h }
        throw (New-InstallError 'health' (T 'ArchieOS started, but {0}. Logs: \\wsl.localhost\{1}{2}' $($h.detail) $($st.distro) $($script:GuestState -replace '/', '\')))
    }
    Set-InstallState $root $st 'running' @{ health = $h; started_at = (Get-Date).ToUniversalTime().ToString('o') }
    return @{ state = 'running'; health = $h }
}
# Stops this distribution only: the database first, cleanly, then wsl --terminate for it.
# WSL itself and the other distributions keep running.
function Stop-ArchieRuntime([string]$root) {
    $st = Get-InstalledState $root
    if ((Get-WslDistroList -Running) -contains $st.distro) {
        $pg = Invoke-Guest $st.distro $script:GuestUser @('/usr/bin/pg_ctl', '-D', "$($script:GuestState)/postgres", 'stop', '-m', 'fast', '-w', '-t', '30') 60
        if ($pg.code -eq 0) { Write-Step 'PostgreSQL detenido' }
        $t = Invoke-Wsl @('--terminate', $st.distro) 60
        if ($t.code -ne 0) { throw (New-InstallError 'stop' "wsl --terminate $($st.distro): $($t.err.Trim())") }
        Write-Step "$($st.distro) detenida"
    }
    Stop-KeepAlive $root $st
    Set-InstallState $root $st 'stopped' @{ health = $null }
    return @{ state = 'stopped' }
}
function Get-RuntimeHealth([string]$root) {
    $st = Get-InstalledState $root
    $h = Test-ArchieHealth $st.distro
    $next = if ($h.ok) { 'running' } elseif (-not $h.wsl) { 'stopped' } else { $st.state }
    if ($next -ne $st.state -and (Test-Transition $st.state $next)) { Set-InstallState $root $st $next @{ health = $h } }
    return @{ state = $st.state; health = $h }
}

# ---- InstallOrchestrator -------------------------------------------------------------------------------------
# The steps after a fresh import that the image's own first start (archie-offline-oobe) does
# when it is installed by double-click: a state folder, a package-signing key of this
# machine's own, and the check of every model in it.
function Initialize-Distro([string]$distro) {
    $r = Invoke-Guest $distro 'root' @('/usr/bin/install', '-d', '-o', 'archie', '-g', 'archie', '-m', '0700', '/home/archie/.local/state') 60
    if ($r.code -ne 0) { throw (New-InstallError 'install' (T 'Could not prepare /home/archie/.local/state: {0}' $($r.err.Trim()))) }
    Write-Step (T 'Generating the package keys for this installation')
    $r = Invoke-Guest $distro 'root' @('/usr/lib/archie/init-pacman-keyring') 600
    if ($r.code -ne 0) { throw (New-InstallError 'install' (T 'init-pacman-keyring failed: {0}' $(($r.err + $r.out).Trim()))) }
}
# archie-offline --check: the image hashes every model it carries, IRIS among them.
function Test-DistroPayload([string]$distro) {
    Write-Step (T 'archie-offline --check: checking the SHA-256 of each model inside the image (IRIS included)')
    $r = Invoke-Guest $distro $script:GuestUser @('/usr/bin/archie-offline', '--check') 1800
    $j = Get-LastJsonLine $r.out
    if ($r.code -ne 0 -or -not $j -or $j.integrity -ne 'ok') {
        throw (New-InstallError 'integrity' "archie-offline --check: $(($r.err + $r.out).Trim())")
    }
    Write-Step (T 'Integrity OK: {0} models, source {1}' $($j.files) $($j.source_sha))
    return $j
}
# IRIS loaded for real: its server, with the weights and the text tower the image carries,
# on the CPU, until it answers /health; then it is stopped. Paths come from the image.
$script:IrisProbe = @'
import json, os, pathlib, signal, socket, subprocess, sys, time, urllib.request
payload = json.loads(pathlib.Path('/usr/share/archie/offline/payload.json').read_text())
weights = payload['files']['iris']
tower = pathlib.Path(weights['path']).parent / 'siglip2-text'
server = '/usr/bin/archie-iris-serve'
if not os.access(server, os.X_OK) or not tower.is_dir():
    print(json.dumps({'iris': 'unavailable', 'detail': 'archie-iris-serve or its text tower are not in the image'})); sys.exit(0)
with socket.socket() as probe:
    if probe.connect_ex(('127.0.0.1', 8765)) == 0:
        print(json.dumps({'iris': 'port_busy', 'detail': 'port 8765 is already used by another process'})); sys.exit(0)
log = open('/tmp/archie-iris-check.log', 'wb')
child = subprocess.Popen([server, '--weights', weights['path'], '--sha256', weights['sha256'], '--text-tower', str(tower),
                          '--port', '8765', '--device', 'cpu'], stdout=log, stderr=log, start_new_session=True)
started = time.monotonic(); answer = None
while time.monotonic() - started < 600 and child.poll() is None:
    try:
        with urllib.request.urlopen('http://127.0.0.1:8765/health', timeout=2) as response:
            answer = response.read().decode()[:200]; break
    except Exception:
        time.sleep(2)
seconds = round(time.monotonic() - started, 1)
if child.poll() is None:
    os.killpg(child.pid, signal.SIGTERM); child.wait(timeout=30)
print(json.dumps({'iris': 'ok' if answer is not None else 'failed', 'seconds': seconds, 'health': answer,
                  'detail': '' if answer is not None else pathlib.Path('/tmp/archie-iris-check.log').read_text(errors='replace')[-400:]}))
'@
function Test-IrisLoad([string]$distro) {
    Write-Step (T 'Loading IRIS on CPU to check that its weights can be read (up to 10 minutes)')
    $r = Invoke-Guest $distro $script:GuestUser @('/usr/bin/python3', '-') 720 $script:IrisProbe
    $j = Get-LastJsonLine $r.out
    if (-not $j) { return [pscustomobject]@{ iris = 'failed'; detail = ($r.err.Trim()) } }
    return $j
}
function Import-Distro([string]$distro, [string]$location, [string]$image) {
    $reg = Get-WslRegistration $distro
    if ($reg) {
        if ($reg.disk_present -and ([IO.Path]::GetFullPath($reg.base_path) -eq [IO.Path]::GetFullPath($location))) {
            Write-Step (T '{0} is already imported at {1}: continuing' $distro $location)
            return
        }
        if (-not $reg.disk_present) { throw (New-InstallError 'repairable' (T '{0} is registered without its disk ({1}). Verify/Repair can remove that broken registration if you confirm.' $distro $($reg.vhdx))) }
        throw (New-InstallError 'distro_exists' (T 'A distribution {0} already exists in {1}. It is not overwritten: reuse it or install under another name.' $distro $($reg.base_path)))
    }
    if (Test-Path -LiteralPath (Join-Path $location 'ext4.vhdx')) {
        throw (New-InstallError 'distro_exists' (T 'There is already an ext4.vhdx disk in {0} that WSL has not registered. It is not touched: move it or choose another folder.' $location))
    }
    New-Item -ItemType Directory -Force $location | Out-Null
    Write-Step (T 'wsl --import {0} (a few minutes; the virtual disk goes to {1})' $distro $location)
    $r = Invoke-Wsl @('--import', $distro, $location, $image, '--version', '2') 7200
    if ($r.code -ne 0) {
        # Nothing was registered: only the folder this step made goes, and only if empty.
        if (-not (Get-ChildItem -LiteralPath $location -Force)) { Remove-Item -LiteralPath $location }
        throw (New-InstallError 'import' (T 'wsl --import failed: {0}' $(($r.err + $r.out).Trim())))
    }
}
function Invoke-Install([string]$root, $manifest, [string]$distroName = '') {
    $target = Get-Target $manifest
    $image = Get-ImageArtifact $manifest
    if (-not $distroName) { $distroName = $target.distro_name }
    Assert-DistroName $distroName
    $st = Read-InstallState $root
    if ($st.state -in @('installed', 'running', 'stopped', 'starting')) {
        Write-Step (T 'ArchieOS is already installed in {0} ({1}).' $root $($st.distro))
        return @{ state = $st.state; distro = $st.distro; already = $true }
    }
    Remove-Item -LiteralPath (Join-Path $root 'state\cancel') -ErrorAction SilentlyContinue
    Set-InstallState $root $st 'checking' @{ release = $manifest.release; target = 'windows-x86_64-wsl2'; distro = $distroName
        location = (Join-Path $root "wsl\$distroName"); error = $null; error_code = $null; job_pid = $PID }
    $probe = Get-HostProbe
    $plan = Get-DependencyPlan $probe $manifest $root $distroName $st
    $blocking = @($plan | Where-Object { $_.blocking })
    if ($blocking.Count) {
        $next = if (@($blocking | Where-Object { $_.status -eq 'reboot_required' }).Count) { 'needs_reboot' } else { 'needs_dependencies' }
        Set-InstallState $root $st $next @{ plan = $plan; job_pid = $null }
        foreach ($b in $blocking) { Write-Step (T 'missing: {0}: {1}' $($b.name) $($b.detail)) }
        return @{ state = $next; plan = $plan }
    }
    $checkpoints = New-Object System.Collections.ArrayList
    if ($st.checkpoints) { $checkpoints.AddRange(@($st.checkpoints)) }
    Set-InstallState $root $st 'downloading' @{ plan = $plan }
    $file = Invoke-ArtifactDownload $image (Join-Path $root 'downloads') $root
    Write-Progress-File $root @{ phase = 'verifying'; artifact = $image.id; received = $image.size_bytes; total = $image.size_bytes }
    Set-InstallState $root $st 'verifying'
    $why = Test-ArtifactFormat $file $image.format
    if ($why) { throw (New-InstallError 'format' (T '{0}: {1}; wsl --import does not accept it.' $($image.file) $why)) }
    Write-Step (T '{0}: gzip-compressed tar, importable with wsl --import' $($image.file))
    $artifacts = @([pscustomobject]@{ id = $image.id; file = $file; sha256 = $image.sha256; size_bytes = $image.size_bytes; version = $image.version
        verified_at = (Get-Date).ToUniversalTime().ToString('o') })
    if ($checkpoints -notcontains 'downloaded') { [void]$checkpoints.Add('downloaded') }
    Set-InstallState $root $st 'installing' @{ artifacts = $artifacts; checkpoints = $checkpoints }
    Write-Progress-File $root @{ phase = 'installing'; artifact = $image.id }
    Import-Distro $distroName $st.location $file
    if ($checkpoints -notcontains 'imported') { [void]$checkpoints.Add('imported') }
    Set-Field $st 'checkpoints' $checkpoints; Save-InstallState $root $st
    try {
        Initialize-Distro $distroName
        $check = Test-DistroPayload $distroName
    } catch {
        Set-InstallState $root $st 'repairable' @{ error = $_.Exception.Message; error_code = (Get-ErrorCode $_.Exception); job_pid = $null }
        throw
    }
    $embedded = @($target.artifacts | Where-Object { $_.embedded_in } | ForEach-Object {
        [pscustomobject]@{ id = $_.id; guest_path = $_.guest_path; sha256 = $_.sha256; size_bytes = $_.size_bytes; version = $_.version; verified_by = 'archie-offline --check' } })
    [void]$checkpoints.Add('initialized')
    Set-InstallState $root $st 'installed' @{ embedded = $embedded; source_sha = $check.source_sha; checkpoints = $checkpoints
        installed_at = (Get-Date).ToUniversalTime().ToString('o'); job_pid = $null }
    Remove-Item -LiteralPath (Join-Path $root 'state\progress.json') -ErrorAction SilentlyContinue
    return @{ state = 'installed'; distro = $distroName; location = $st.location; release = $manifest.release }
}
# Verificar: the registration, its disk and archie-offline --check; with -Deep, IRIS loaded.
function Invoke-Verify([string]$root, [switch]$Deep) {
    $st = Get-InstalledState $root
    $was = $st.state
    $wasRunning = (Get-WslDistroList -Running) -contains $st.distro
    Set-InstallState $root $st 'checking'
    try { $check = Test-DistroPayload $st.distro }
    catch { Set-InstallState $root $st 'repairable' @{ error = $_.Exception.Message; error_code = 'integrity' }; throw }
    $iris = if ($Deep) { Test-IrisLoad $st.distro } else { $null }
    if ($iris) { Write-Step ("IRIS: {0} {1}" -f $iris.iris, $(if ($iris.seconds) { "en $($iris.seconds) s" } else { $iris.detail })) }
    $h = Test-ArchieHealth $st.distro
    $next = if ($h.ok) { 'running' } elseif ($was -eq 'installed') { 'installed' } else { 'stopped' }
    Set-InstallState $root $st $next @{ verified_at = (Get-Date).ToUniversalTime().ToString('o'); iris_check = $iris; error = $null; error_code = $null }
    # A check that started the distribution leaves it as it found it.
    if (-not $wasRunning -and -not $h.ok) { [void](Invoke-Wsl @('--terminate', $st.distro) 60) }
    return @{ state = $next; integrity = $check; iris = $iris; health = $h }
}
# Reparar: puts back what is missing without deleting anything of the person's. A
# registration whose disk is gone is removed only with -AllowUnregisterBroken (the person
# confirmed) and only after checking again that the disk really is not there.
function Invoke-Repair([string]$root, $manifest, [switch]$AllowUnregisterBroken) {
    $st = Read-InstallState $root
    if (-not $st.distro) { throw (New-InstallError 'not_installed' (T 'There is no installation to repair in this folder.')) }
    $reg = Get-WslRegistration $st.distro
    if ($reg -and -not $reg.disk_present) {
        if (-not $AllowUnregisterBroken) { throw (New-InstallError 'confirm' (T '{0} is registered without its disk ({1}). Repair needs your confirmation to remove that registration.' $($st.distro) $($reg.vhdx))) }
        if (Test-Path -LiteralPath $reg.vhdx) { throw (New-InstallError 'confirm' (T 'The disk appeared: nothing is removed.')) }
        Write-Step (T 'wsl --unregister {0} (registration without a disk, confirmed)' $($st.distro))
        $u = Invoke-Wsl @('--unregister', $st.distro) 120
        if ($u.code -ne 0) { throw (New-InstallError 'repair' "wsl --unregister: $($u.err.Trim())") }
        $reg = $null
    }
    Set-InstallState $root $st 'checking'
    if (-not $reg) {
        Set-InstallState $root $st 'repairable' @{ error = $null }
        Set-InstallState $root $st 'installing'
        $image = Get-ImageArtifact $manifest
        $file = Invoke-ArtifactDownload $image (Join-Path $root 'downloads') $root
        Import-Distro $st.distro $st.location $file
    } elseif ([IO.Path]::GetFullPath($reg.base_path) -ne [IO.Path]::GetFullPath($st.location) -and -not $st.adopted) {
        throw (New-InstallError 'distro_exists' (T '{0} exists in {1}, which does not belong to this installation. It is not touched.' $($st.distro) $($reg.base_path)))
    } else {
        Set-InstallState $root $st 'repairable' @{ error = $null }
        Set-InstallState $root $st 'installing'
    }
    Initialize-Distro $st.distro
    $check = Test-DistroPayload $st.distro
    Set-InstallState $root $st 'installed' @{ source_sha = $check.source_sha; repaired_at = (Get-Date).ToUniversalTime().ToString('o'); error = $null; error_code = $null }
    return @{ state = 'installed'; integrity = $check }
}
# Reutilizar: a distribution already there becomes this installation's, if the image's own
# check says it is ArchieOS with every model intact. Nothing is imported or replaced.
function Invoke-Adopt([string]$root, [string]$distro) {
    Assert-DistroName $distro
    $reg = Get-WslRegistration $distro
    if (-not $reg -or -not $reg.disk_present) { throw (New-InstallError 'not_found' (T 'There is no usable distribution {0}.' $distro)) }
    $st = Read-InstallState $root
    Set-InstallState $root $st 'checking' @{ distro = $distro; location = $reg.base_path; adopted = $true }
    $check = Test-DistroPayload $distro
    Initialize-Distro $distro
    Set-InstallState $root $st 'installed' @{ source_sha = $check.source_sha; installed_at = (Get-Date).ToUniversalTime().ToString('o') }
    return @{ state = 'installed'; distro = $distro; location = $reg.base_path }
}
function Get-InstallStatus([string]$root) {
    $st = Read-InstallState $root
    $reg = if ($st.distro) { Get-WslRegistration $st.distro } else { $null }
    $job = $false
    if ($st.job_pid) { $job = [bool](Get-Process -Id $st.job_pid -ErrorAction SilentlyContinue | Where-Object { $_.ProcessName -eq 'powershell' }) }
    return @{ state = $st.state; install = $st; registration = $reg; job_running = $job }
}

# ---- command line ---------------------------------------------------------------------------------------------
function Write-Result([hashtable]$result) {
    $json = ConvertTo-Json $result -Compress -Depth 8
    if ($ResultFile) { [System.IO.File]::WriteAllText($ResultFile, "$json`n", $script:Utf8) }
    elseif ($script:OutPath) { [System.IO.File]::AppendAllText($script:OutPath, "$json`n", $script:Utf8) } else { Write-Output $json }
}
if ($Verb) {
    $ErrorActionPreference = 'Stop'
    $script:OutPath = $OutFile
    if ($Root) {
        $Root = [System.IO.Path]::GetFullPath($Root)
        New-Item -ItemType Directory -Force (Join-Path $Root 'logs'), (Join-Path $Root 'state') | Out-Null
        $script:LogPath = Join-Path $Root 'logs\installer.log'
    }
    $exit = 0
    try {
        $needsRoot = $Verb -notin @('probe', 'plan', 'fix-dependency')
        if ($needsRoot -and -not $Root) { throw (New-InstallError 'invalid' "$Verb necesita -Root") }
        $manifest = Read-ReleaseManifest $ManifestPath
        $result = switch ($Verb) {
            'probe' { @{ probe = (Get-HostProbe) } }
            'plan' {
                $probe = Get-HostProbe
                $st = if ($Root) { Read-InstallState $Root } else { $null }
                if ($Root) { Update-RebootState $Root $st $probe }
                @{ probe = $probe; plan = @(Get-DependencyPlan $probe $manifest $Root $Distro $st); state = $(if ($st) { $st.state } else { 'not_installed' })
                    required = $(if ($Root) { Get-RequiredBytes $Root (Get-ImageArtifact $manifest) (Get-Target $manifest) $st }) }
            }
            'install' { Invoke-Install $Root $manifest $Distro }
            'verify' { Invoke-Verify $Root -Deep:$Deep }
            'repair' { Invoke-Repair $Root $manifest -AllowUnregisterBroken:$AllowUnregisterBroken }
            'adopt' { Invoke-Adopt $Root $Distro }
            'start' { Start-ArchieRuntime $Root }
            'stop' { Stop-ArchieRuntime $Root }
            'health' { Get-RuntimeHealth $Root }
            'status' { Get-InstallStatus $Root }
            'fix-dependency' { Invoke-DependencyFix $Id $Root }
            default { throw (New-InstallError 'invalid' "Verbo desconocido: $Verb") }
        }
        $result['ok'] = $true
        Write-Result $result
    } catch {
        $code = Get-ErrorCode $_.Exception
        $message = $_.Exception.Message -replace '^\[\w+\]\s*', ''
        Write-Step "Error: $message"
        if ($Root) {
            # The state says what went wrong, unless the step already left a better one.
            $st = Read-InstallState $Root
            if ($st.state -notin @('not_installed', 'repairable', 'needs_dependencies', 'needs_reboot', 'error')) {
                try { Set-InstallState $Root $st 'error' @{ error = $message; error_code = $code; job_pid = $null } } catch { }
            }
        }
        Write-Result @{ ok = $false; error = $message; code = $code }
        $exit = 1
    }
    exit $exit
}
