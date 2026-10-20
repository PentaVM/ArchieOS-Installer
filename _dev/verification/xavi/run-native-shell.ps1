param(
    [Parameter(Mandatory)][string]$NativeBin,
    [Parameter(Mandatory)][string]$BridgeBin,
    [Parameter(Mandatory)][string]$DisplaydBin,
    [Parameter(Mandatory)][string]$QtBin,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [ValidateSet('dark','light')][string[]]$Themes=@('dark','light')
)
$ErrorActionPreference='Stop'
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
foreach($port in @(7666,7668)) {
    $check=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,$port)
    try { $check.Start() } finally { $check.Stop() }
}
$env:PATH=$QtBin+';'+$env:PATH
$env:QT_QPA_PLATFORM='windows'
$env:QT_QUICK_CONTROLS_STYLE='Basic'
$env:QSG_RHI_BACKEND='opengl'
$env:QSG_INFO='1'
Remove-Item Env:QT_QUICK_BACKEND -ErrorAction SilentlyContinue
$fixture=Join-Path $PSScriptRoot 'native-shell-fixture.json'
$server=$null; $window=$null
try {
    $server=Start-Process -FilePath $DisplaydBin -ArgumentList "--fixture `"$fixture`" --disable-world-feed" -WindowStyle Hidden -PassThru -RedirectStandardError (Join-Path $OutputDirectory 'displayd.stderr') -RedirectStandardOutput (Join-Path $OutputDirectory 'displayd.stdout')
    Start-Sleep -Seconds 1
    $inspection=Invoke-RestMethod -Uri 'http://127.0.0.1:7666/api/inspector' -TimeoutSec 3
    if($inspection.source -ne 'fixture'){throw 'Displayd must identify the fixture'}
    foreach($theme in $Themes) {
        $png=Join-Path $OutputDirectory ($theme+'.png')
        if(Test-Path -LiteralPath $png){throw "Output already exists: $png"}
        $errorLog=Join-Path $OutputDirectory ($theme+'.stderr')
        # This is the interactive GUI under test. It must be exposed: a hidden window
        # intentionally receives a zero render budget. The backend helper stays hidden.
        $window=Start-Process -FilePath $NativeBin -ArgumentList "--shell --screen 0 --theme $theme --shell-bridge `"$BridgeBin`" --render-check `"$png`"" -WindowStyle Normal -PassThru -RedirectStandardError $errorLog -RedirectStandardOutput (Join-Path $OutputDirectory ($theme+'.stdout'))
        $deadline=[DateTime]::UtcNow.AddSeconds(15)
        $client=$null
        do {
            if($window.HasExited){throw "Native window exited during startup: $($window.ExitCode)"}
            $probe=[Net.Sockets.TcpClient]::new()
            try { $probe.Connect('127.0.0.1',7668); $client=$probe }
            catch { $probe.Dispose(); Start-Sleep -Milliseconds 200 }
        } while(-not $client -and [DateTime]::UtcNow -lt $deadline)
        if(-not $client){throw 'Native control did not become ready within 15 seconds'}
        try {
            $client.ReceiveTimeout=1500
            $line=if($theme -eq 'dark'){'{"action":"cinematic","value":"crown"}'}else{'{"action":"panel","value":"settings"}'}
            $bytes=[Text.Encoding]::UTF8.GetBytes($line+"`n")
            $stream=$client.GetStream(); $stream.Write($bytes,0,$bytes.Length)
            $reader=[IO.StreamReader]::new($stream)
            if($reader.ReadLine() -ne 'ok'){throw 'Native display rejected the visual route'}
        } finally { $client.Dispose() }
        if(-not $window.WaitForExit(20000)){throw 'Native render timed out'}
        if($window.ExitCode -ne 0){throw "Native render failed: $($window.ExitCode)"}
        $log=Get-Content -LiteralPath $errorLog -Raw
        if($log -notmatch 'bridge=fixture child-running=true'){throw 'Real Rust child did not reach QML as fixture'}
        if($log -notmatch 'orb=(\{[^\r\n]+\}) bridge-presented=true'){throw 'Native viewport must be presented for the render test'}
        $orb=$Matches[1] | ConvertFrom-Json
        if(-not ($orb.owned -and $orb.viewportPresented -and $orb.controllerActive)){throw 'Native Orb did not acquire the presented viewport'}
        # Snapshot fixture mode deliberately does not feed authoritative TFOG entities.
        # Empty Orb is expected here; native_contracts separately verifies valid scene ingestion.
        if($log -match 'Error compiling|ReferenceError|TypeError|Unable to assign|not functional|failed to load'){throw 'Native QML/render diagnostics require review'}
        Get-Item -LiteralPath $png | Select-Object Name,Length
        $window=$null
    }
} finally {
    if($window -and -not $window.HasExited){Stop-Process -Id $window.Id}
    if($server -and -not $server.HasExited){Stop-Process -Id $server.Id}
}
