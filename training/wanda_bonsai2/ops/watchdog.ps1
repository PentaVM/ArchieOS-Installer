# Windows-side guardian for the overnight Wanda worker.
# - Asks Windows not to idle-sleep while THIS process runs (SetThreadExecutionState;
#   no power setting is changed, the request ends with the process).
# - Keeps one WSL session open and restarts the worker if it died; the worker
#   resumes its queued job from the last atomic checkpoint.
# - Exits at the cutoff. Stop early: write {"command":"stop"} to the run root's
#   control.json, then end this process.
param(
  [Parameter(Mandatory = $true)][string]$Cutoff,
  [string]$Distro = 'ArchieOS-Operator-RTX3070-Test',
  [string]$Script = '/mnt/c/Users/Penta/.codex/worktrees/wanda-bonsai9b/ArchieOS/training/wanda_bonsai2/ops/ensure-worker.sh',
  [string]$Log = "$env:USERPROFILE\Archie-RTX3070-Handoff\wanda-overnight\watchdog.log"
)
$ErrorActionPreference = 'Continue'
New-Item -ItemType Directory -Force (Split-Path $Log) | Out-Null
Add-Type -Namespace Archie -Name Power -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("kernel32.dll")]
public static extern uint SetThreadExecutionState(uint flags);
'@
$ES_CONTINUOUS = [uint32]2147483648; $ES_SYSTEM_REQUIRED = [uint32]1
$end = [DateTimeOffset]::Parse($Cutoff)
"$(Get-Date -Format o) watchdog start pid=$PID cutoff=$Cutoff" | Out-File -Append -Encoding utf8 $Log
while ([DateTimeOffset]::Now -lt $end) {
  [void][Archie.Power]::SetThreadExecutionState($ES_CONTINUOUS -bor $ES_SYSTEM_REQUIRED)
  & wsl.exe -d $Distro --cd / -e bash $Script $Cutoff 240
  if ($LASTEXITCODE -ne 0) {
    "$(Get-Date -Format o) ensure-worker exit $LASTEXITCODE" | Out-File -Append -Encoding utf8 $Log
    Start-Sleep -Seconds 30
  }
}
[void][Archie.Power]::SetThreadExecutionState($ES_CONTINUOUS)
"$(Get-Date -Format o) watchdog end (cutoff)" | Out-File -Append -Encoding utf8 $Log
