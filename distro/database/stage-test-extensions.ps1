$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$dependencyDir = Join-Path $repoRoot '.dashboard-local/dependencies'
$archivePath = Join-Path $dependencyDir 'timescaledb-pg18-2.29.2.zip'
if ((Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLower() -ne 'cc80949c34680a28e5694f7ad9cc3948a8d457c09efef43082fdc969959295a8') { throw 'Timescale release checksum mismatch' }
$vectorRoot = Join-Path $dependencyDir 'pgvector-0.8.6'
if ((git -C $vectorRoot rev-parse HEAD).Trim() -ne '8ee86c96f0fd72390f890aa8a336fda6d3ab4c6c') { throw 'pgvector source revision mismatch' }
if (git -C $vectorRoot status --porcelain --untracked-files=no) { throw 'pgvector tracked source has changes' }
$runtimeRoot = Join-Path $dependencyDir 'postgres18-extensions'
$libRoot = Join-Path $runtimeRoot 'lib'
$controlRoot = Join-Path $runtimeRoot 'share/extension'
New-Item -ItemType Directory -Path $libRoot,$controlRoot -Force | Out-Null
$timescaleRoot = Join-Path $dependencyDir 'timescaledb-pg18-2.29.2/timescaledb'
Copy-Item -LiteralPath (Join-Path $timescaleRoot 'timescaledb.dll'),(Join-Path $timescaleRoot 'timescaledb-2.29.2.dll'),(Join-Path $timescaleRoot 'timescaledb-tsl-2.29.2.dll'),(Join-Path $vectorRoot 'vector.dll') -Destination $libRoot
$libraryPrefix = $libRoot.Replace('\','/').Replace("'","''")
# PostgreSQL 18 supports a separate extension search path. Rewrite library paths
# only in fixture copies; never modify the downloaded originals or system install.
foreach ($fileName in @('timescaledb.control','timescaledb--2.29.2.sql')) {
    $content = Get-Content -LiteralPath (Join-Path $timescaleRoot $fileName) -Raw
    [IO.File]::WriteAllText((Join-Path $controlRoot $fileName),$content.Replace('$libdir/',$libraryPrefix+'/'))
}
foreach ($fileName in @('vector.control','sql/vector--0.8.6.sql')) {
    $content = Get-Content -LiteralPath (Join-Path $vectorRoot $fileName) -Raw
    [IO.File]::WriteAllText((Join-Path $controlRoot ([IO.Path]::GetFileName($fileName))),$content.Replace('$libdir/',$libraryPrefix+'/'))
}
Write-Output 'Isolated test extensions staged. No system PostgreSQL files changed.'
