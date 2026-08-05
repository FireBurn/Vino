# Runbook phase 7b.2 -- does 10-bit change the dock's link/bandwidth budget?
#
# 10-bit needs more link bandwidth than 8-bit at the same mode, so if the dock's pixel
# budget is depth-aware the set-mode words should move when HDR is on at a higher refresh.
# The capture holds all four combinations in one file so they can be diffed directly:
#
#   60 Hz SDR -> 60 Hz HDR -> 120 Hz HDR -> 120 Hz SDR -> back to 60 Hz
#
# Only the head under test changes refresh; the other head stays at 60 Hz SDR as a control.
#
# Nothing here goes above 120 Hz. 180 is what knocked this dock into a reconnect loop in
# the first place, and the mode ceiling is already known from cap4-modesweep.
param(
    [string]$Head  = '\\.\DISPLAY29',
    [string]$Other = '\\.\DISPLAY30',
    [string]$Root  = 'C:\Users\Mike\navarro-wincap',
    [int]$MaxSeconds = 300
)

$ErrorActionPreference = 'Stop'

$out       = Join-Path $Root 'out'
$jobs      = Join-Path $out  'jobs'
$hdrPs     = Join-Path $Root 'tools\hdr.ps1'
$cdpPs     = Join-Path $Root 'tools\cdp.ps1'
$refreshPs = Join-Path $Root 'tools\rescue-refresh.ps1'
$psexe     = (Get-Command powershell.exe).Source

$OutPrefix = Join-Path $out 'cap14-mode120'
$logPath   = "$OutPrefix.phaselog.txt"

function Log([string]$t) {
    $line = "{0}  {1}" -f (Get-Date -Format 'yyyy-MM-dd HH:mm:ss.fff'), $t
    Write-Host $line
    Add-Content -Path $logPath -Value $line -Encoding utf8
}
function Cdp([string]$js, [switch]$Gesture) {
    $a = @('-NoProfile','-ExecutionPolicy','Bypass','-File',$cdpPs,'-Js',$js)
    if ($Gesture) { $a += '-UserGesture' }
    (& $psexe @a 2>&1 | Out-String).Trim()
}
function Hdr([string]$d, [bool]$on, [string]$tag) {
    $a = @('-NoProfile','-ExecutionPolicy','Bypass','-File',$hdrPs,'-Display',$d,
           $(if ($on) { '-On' } else { '-Off' }),'-Tag',$tag)
    Log ("HDR     " + (& $psexe @a 2>&1 | Out-String).Trim())
}
function Set-Hz([int]$hz) {
    Log "MODE    setting $Head to $hz Hz"
    # Splat the argument array: `& $exe 'a','b'` passes one comma-joined argument, not two.
    $setArgs = @('-NoProfile','-ExecutionPolicy','Bypass','-File',$refreshPs,
                 '-Hz',"$hz",'-Device',$Head,'-NoDisplaySwitch','-TimeoutSeconds','20')
    $r = & $psexe @setArgs 2>&1
    ($r | Out-String).Trim().Split("`n") | Where-Object { $_ -match 'attempt|apply|!!' } |
        ForEach-Object { Log "  | $($_.Trim())" }
    Start-Sleep -Seconds 4
    $listArgs = @('-NoProfile','-ExecutionPolicy','Bypass','-File',$refreshPs,'-List')
    $now = & $psexe @listArgs 2>&1 | Out-String
    $m = [regex]::Match($now, [regex]::Escape($Head) + '.*?current\s*:\s*(\d+)x(\d+) @ (\d+) Hz', 'Singleline')
    if ($m.Success) { Log "MODE    $Head now $($m.Groups[1].Value)x$($m.Groups[2].Value) @ $($m.Groups[3].Value) Hz" }
    else            { Log "MODE    (could not read back $Head)" }
}

Log '=== phase mode120 (runbook 7b.2) ==='
if (-not (Test-Path (Join-Path $jobs 'runner.ready'))) { throw 'elevated capture runner is not up' }

[void](Cdp "(()=>{for(const id of ['info','log','hint'])document.getElementById(id).classList.add('hidden');return 'ok';})()")

$name = [IO.Path]::GetFileName($OutPrefix)
Remove-Item "$jobs\$name.finished","$jobs\$name.started" -ErrorAction SilentlyContinue
Set-Content -Path "$jobs\$name.job" -Encoding utf8 -Value (
    @{ OutPrefix = $OutPrefix; Snaplen = 4096; MaxSeconds = $MaxSeconds
       BufferLen = 134217728 } | ConvertTo-Json)
$t0 = Get-Date
while (-not (Test-Path "$jobs\$name.started") -and ((Get-Date) - $t0).TotalSeconds -lt 30) {
    Start-Sleep -Milliseconds 200
}
if (-not (Test-Path "$jobs\$name.started")) { throw 'runner never started the job' }
Log "CAPTURE started ($name)"
Start-Sleep -Seconds 3

Hdr $Other $false 'mode120: other head stays 60 Hz SDR (control)'
Hdr $Head  $false 'mode120: baseline SDR'
Set-Hz 60

Log 'PLAY    hdr-pattern @ 60 Hz SDR  30 s'
[void](Cdp "(()=>{load(SOURCES[0]);return 'ok';})()" -Gesture)
Start-Sleep -Seconds 30

Hdr $Head $true 'mode120: HDR ON at 60 Hz'
Start-Sleep -Seconds 30
Log 'STATE   60 Hz HDR, pattern playing'

Set-Hz 120
Start-Sleep -Seconds 2
$c = Cdp "(()=>{const v=document.getElementById('v');return JSON.stringify({hi:window.matchMedia('(dynamic-range: high)').matches,depth:screen.colorDepth,dpr:window.devicePixelRatio,paused:v.paused});})()"
Log "PLAYER  at 120 Hz  $c"
Start-Sleep -Seconds 32
Log 'STATE   120 Hz HDR, pattern playing'

Hdr $Head $false 'mode120: HDR OFF still at 120 Hz'
Start-Sleep -Seconds 30
Log 'STATE   120 Hz SDR, pattern playing'

# Leave the dock at 60 Hz. This is the one thing the runbook is emphatic about.
Set-Hz 60
Start-Sleep -Seconds 15

Set-Content -Path (Join-Path $out 'stop.flag') -Value 'x' -Encoding utf8
Log 'CAPTURE stop.flag written'
$t0 = Get-Date
while (-not (Test-Path "$jobs\$name.finished") -and ((Get-Date) - $t0).TotalSeconds -lt 90) {
    Start-Sleep -Milliseconds 500
}
if (Test-Path "$jobs\$name.finished") {
    Log 'CAPTURE finished'
    Get-Content "$jobs\$name.finished" | ForEach-Object { Log "  $_" }
} else { Log '!! runner did not report finished' }
Remove-Item (Join-Path $out 'stop.flag') -ErrorAction SilentlyContinue

Log '=== phase mode120 complete ==='
Write-Host ''
Write-Host "phase log: $logPath"
