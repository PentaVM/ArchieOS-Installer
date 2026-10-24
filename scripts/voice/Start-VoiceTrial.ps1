param(
    [string]$Distro = 'ArchieOS-Operator-RTX3070-Test',
    [string]$Launcher = '/home/archie/.local/share/archie-voice-floor-trial/run',
    [string]$Endpoint = 'http://127.0.0.1:8081/v1',
    [string]$Model = 'bonsai',
    [switch]$Speakers,
    [switch]$Always
)

$ErrorActionPreference = 'Stop'
Write-Host 'Prueba aislada de voz/floor. Usa el Bonsai local existente; no inicia modelos ni entrenamiento.'
Write-Host 'Di «Archie» para hablar. Ctrl+C termina esta sesión. No tiene acciones de escritorio conectadas.'
if ($Speakers) {
    Write-Host 'Altavoces: el micrófono se cierra mientras Archie habla porque aún no hay cancelación de eco.'
} else {
    Write-Host 'Usa audífonos para probar interrupciones sin que el micrófono oiga la propia voz.'
}
$trialArgs = @('--endpoint', $Endpoint, '--model', $Model)
if (-not $Speakers) { $trialArgs += '--headphones' }
if ($Always) {
    Write-Host 'Modo individual: responderá a cualquier petición, aunque el ASR no reconozca «Archie».'
    $trialArgs += '--always'
}
& "$env:WINDIR\System32\wsl.exe" -d $Distro --exec $Launcher @trialArgs
if ($LASTEXITCODE -ne 0) {
    throw "La sesión de prueba terminó con código $LASTEXITCODE. Revisa el diagnóstico mostrado arriba."
}
