# Contract check only: simulated WSL import, no distro or desktop is created.
$ErrorActionPreference = 'Stop'
$global:ArchieKeyringContractCalls = [System.Collections.Generic.List[string]]::new()
function global:wsl.exe {
    $call = $args -join ' '
    $global:ArchieKeyringContractCalls.Add($call)
    $global:LASTEXITCODE = 0
    if ($call -like '*init-pacman-keyring*') { $global:LASTEXITCODE = 17 }
}
$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('archie-install-keyring-' + [guid]::NewGuid())
try {
    New-Item -ItemType Directory -Path $testRoot | Out-Null
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'install-operator.ps1') -Destination $testRoot
    Set-Content -LiteralPath (Join-Path $testRoot 'ArchieOS-Operator.wsl') -Value 'fixture'
    $failed = $false
    try {
        & (Join-Path $testRoot 'install-operator.ps1') -Name 'KeyringContractFixture' -InstallDir (Join-Path $testRoot 'import')
    } catch {
        if ($_.Exception.Message -notlike '*keyring initialization failed*') { throw }
        $failed = $true
    }
    if (-not $failed) { throw 'Installer continued after keyring failure' }
    if ($global:ArchieKeyringContractCalls.Count -ne 3) { throw "Unexpected WSL calls: $($global:ArchieKeyringContractCalls -join '; ')" }
    # PowerShell consumes -- when a function intercepts the native command.
    if ($global:ArchieKeyringContractCalls[2].Replace(' -- ', ' ') -ne '-d KeyringContractFixture -u root /usr/lib/archie/init-pacman-keyring') {
        throw 'Installer did not initialize as root immediately after import'
    }
    Write-Output 'PASS: import initializes keyring as root and stops on failure.'
} finally {
    Remove-Item Function:\wsl.exe -ErrorAction SilentlyContinue
    Remove-Variable -Name ArchieKeyringContractCalls -Scope Global -ErrorAction SilentlyContinue
    $resolved = [IO.Path]::GetFullPath($testRoot)
    $tempParent = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
    if (-not $resolved.StartsWith($tempParent, [StringComparison]::OrdinalIgnoreCase)) { throw 'Unsafe test cleanup path' }
    if (Test-Path -LiteralPath $resolved) { Remove-Item -LiteralPath $resolved -Recurse -Force }
}
