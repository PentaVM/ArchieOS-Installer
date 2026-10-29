# Signs and verifies an ArchieOS release with ECDSA P-256 / SHA-256.
#
#   Sign-Release.ps1 keygen                       one-time: makes the release key
#   Sign-Release.ps1 sign   -Manifest FILE        writes FILE.sig (and signs SHA256SUMS if -Sums)
#   Sign-Release.ps1 verify -Manifest FILE        checks FILE against FILE.sig and the public key
#
# The private key never leaves this Windows account: it is sealed with DPAPI (CurrentUser) in
# %LOCALAPPDATA%\ArchieOS\release-signing. Back it up with `export` if you change machines;
# losing it means shipping a new public key in a new installer. The public key is what the
# installer trusts (distro/windows/ArchieOS-Installer.ps1, $script:TrustedKeys).
#
# The signature is the raw IEEE P1363 pair (r||s, 64 bytes) over the exact bytes of the signed
# file, in a small JSON envelope, so `verify` and the installer need nothing beyond
# Windows PowerShell 5.1. Windows PowerShell 5.1, no module to install.
param(
    [Parameter(Mandatory, Position = 0)][ValidateSet('keygen', 'sign', 'verify', 'export')][string]$Verb,
    [string]$Manifest = '',
    [string]$Sums = '',
    [string]$KeyDir = (Join-Path $env:LOCALAPPDATA 'ArchieOS\release-signing'),
    [string]$OutFile = ''
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Security
$privatePath = Join-Path $KeyDir 'release-p256.key.dpapi'
$publicPath = Join-Path $KeyDir 'release-p256.pub.json'

function Get-KeyId([byte[]]$publicBlob) {
    $sha = [Security.Cryptography.SHA256]::Create()
    return (($sha.ComputeHash($publicBlob) | Select-Object -First 8 | ForEach-Object { $_.ToString('x2') }) -join '')
}
function New-ReleaseKey {
    if (Test-Path -LiteralPath $privatePath) { throw "A release key already exists in $KeyDir. Refusing to replace it." }
    New-Item -ItemType Directory -Force $KeyDir | Out-Null
    $cng = [Security.Cryptography.CngKey]::Create([Security.Cryptography.CngAlgorithm]::ECDsaP256, $null,
        (New-Object Security.Cryptography.CngKeyCreationParameters -Property @{ ExportPolicy = [Security.Cryptography.CngExportPolicies]::AllowPlaintextExport }))
    $private = $cng.Export([Security.Cryptography.CngKeyBlobFormat]::EccPrivateBlob)
    $public = $cng.Export([Security.Cryptography.CngKeyBlobFormat]::EccPublicBlob)
    $sealed = [Security.Cryptography.ProtectedData]::Protect($private, $null, 'CurrentUser')
    [IO.File]::WriteAllBytes($privatePath, $sealed)
    $info = [ordered]@{ alg = 'ecdsa-p256-sha256'; key_id = (Get-KeyId $public); public_blob = [Convert]::ToBase64String($public)
        created = (Get-Date).ToUniversalTime().ToString('o') }
    ($info | ConvertTo-Json) | Set-Content -LiteralPath $publicPath -Encoding UTF8
    Write-Host "Release key $($info.key_id) made in $KeyDir"
    Write-Host "Public key (goes in the installer's TrustedKeys):"
    Write-Host "  key_id      $($info.key_id)"
    Write-Host "  public_blob $($info.public_blob)"
}
function Get-SigningKey {
    if (-not (Test-Path -LiteralPath $privatePath)) { throw "No release key in $KeyDir. Run: Sign-Release.ps1 keygen" }
    $private = [Security.Cryptography.ProtectedData]::Unprotect([IO.File]::ReadAllBytes($privatePath), $null, 'CurrentUser')
    return New-Object Security.Cryptography.ECDsaCng ([Security.Cryptography.CngKey]::Import($private, [Security.Cryptography.CngKeyBlobFormat]::EccPrivateBlob))
}
function Get-PublicInfo { return (Get-Content -LiteralPath $publicPath -Raw -Encoding UTF8 | ConvertFrom-Json) }
function New-Signature([string]$file) {
    $ec = Get-SigningKey
    $ec.HashAlgorithm = [Security.Cryptography.CngAlgorithm]::Sha256
    $bytes = [IO.File]::ReadAllBytes($file)
    $sig = $ec.SignData($bytes)
    $info = Get-PublicInfo
    $sha = [Security.Cryptography.SHA256]::Create()
    $env_ = [ordered]@{ alg = 'ecdsa-p256-sha256'; key_id = $info.key_id; file = (Split-Path $file -Leaf)
        sha256 = (($sha.ComputeHash($bytes) | ForEach-Object { $_.ToString('x2') }) -join '')
        signed_at = (Get-Date).ToUniversalTime().ToString('o'); signature = [Convert]::ToBase64String($sig) }
    $out = "$file.sig"
    [IO.File]::WriteAllText($out, (($env_ | ConvertTo-Json) + "`n"), (New-Object Text.UTF8Encoding($false)))
    Write-Host "Signed $(Split-Path $file -Leaf) with key $($info.key_id) -> $out"
}
function Test-Signature([string]$file) {
    $sigFile = "$file.sig"
    if (-not (Test-Path -LiteralPath $sigFile)) { throw "No signature file $sigFile" }
    $s = Get-Content -LiteralPath $sigFile -Raw -Encoding UTF8 | ConvertFrom-Json
    $info = Get-PublicInfo
    if ($s.key_id -ne $info.key_id) { throw "Signed by key $($s.key_id), not $($info.key_id)" }
    $key = [Security.Cryptography.CngKey]::Import([Convert]::FromBase64String($info.public_blob), [Security.Cryptography.CngKeyBlobFormat]::EccPublicBlob)
    $ec = New-Object Security.Cryptography.ECDsaCng $key
    $ec.HashAlgorithm = [Security.Cryptography.CngAlgorithm]::Sha256
    if (-not $ec.VerifyData([IO.File]::ReadAllBytes($file), [Convert]::FromBase64String($s.signature))) { throw "BAD signature on $file" }
    Write-Host "OK: $(Split-Path $file -Leaf) is signed by key $($s.key_id)"
}

switch ($Verb) {
    'keygen' { New-ReleaseKey }
    'sign' {
        if (-not $Manifest) { throw '-Manifest is required' }
        New-Signature (Resolve-Path -LiteralPath $Manifest).Path
        if ($Sums) { New-Signature (Resolve-Path -LiteralPath $Sums).Path }
    }
    'verify' {
        if (-not $Manifest) { throw '-Manifest is required' }
        Test-Signature (Resolve-Path -LiteralPath $Manifest).Path
    }
    'export' {
        # An encrypted-at-rest backup is the operator's decision: this prints the public half only.
        Get-Content -LiteralPath $publicPath -Raw
    }
}
