# Long-lived elevated capture runner.
#
# USBPcapCMD needs administrator rights, and this session takes five captures. Rather than
# raise a UAC prompt per capture -- which puts a consent dialog in the middle of a timed
# choreography and makes the log times a lie -- this runs elevated once and then takes work
# from a directory.
#
# Start it (this is the one and only UAC prompt):
#   Start-Process powershell -Verb RunAs -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass',
#       '-File','C:\Users\Mike\navarro-wincap\tools\capture-runner.ps1'
#
# Then to run a capture, drop a job file in out\jobs\ :
#   { "OutPrefix": "C:\\...\\out\\cap9-hdr-ab", "Snaplen": 0, "MaxSeconds": 420,
#     "BufferLen": 134217728 }
#
# The runner writes out\jobs\<name>.started and <name>.finished so an unelevated caller can
# follow along, and honours the same out\stop.flag sentinel capture-both.ps1 already uses.
# Drop out\jobs\quit.flag to make it exit.
param(
    [string]$Root = 'C:\Users\Mike\navarro-wincap',
    [int]$PollMs = 250
)

$ErrorActionPreference = 'Continue'

$out     = Join-Path $Root 'out'
$jobs    = Join-Path $out  'jobs'
$capture = Join-Path $Root 'tools\capture-both.ps1'
$runlog  = Join-Path $jobs 'runner.log'

New-Item -ItemType Directory -Force -Path $jobs | Out-Null

function Log([string]$m) {
    $line = "{0}  {1}" -f (Get-Date -Format 'yyyy-MM-dd HH:mm:ss.fff'), $m
    Write-Host $line
    Add-Content -Path $runlog -Value $line -Encoding utf8
}

$id = [Security.Principal.WindowsIdentity]::GetCurrent()
$elevated = (New-Object Security.Principal.WindowsPrincipal($id)).IsInRole(
                [Security.Principal.WindowsBuiltInRole]::Administrator)

Log "=== capture-runner up ==="
Log "elevated : $elevated"
Log "watching : $jobs"
if (-not $elevated) {
    Log "!! NOT ELEVATED -- USBPcap will fail with access denied. Restart me with -Verb RunAs."
}

# An unelevated caller polls for this to know the runner is alive and privileged.
Set-Content -Path (Join-Path $jobs 'runner.ready') -Encoding utf8 -Value @"
pid=$PID
elevated=$elevated
started=$(Get-Date -Format 'yyyy-MM-dd HH:mm:ss.fff')
"@

$quit = Join-Path $jobs 'quit.flag'

while (-not (Test-Path $quit)) {
    $job = Get-ChildItem -Path $jobs -Filter '*.job' -ErrorAction SilentlyContinue |
           Sort-Object CreationTime | Select-Object -First 1
    if (-not $job) { Start-Sleep -Milliseconds $PollMs; continue }

    $name = [IO.Path]::GetFileNameWithoutExtension($job.Name)
    $spec = $null
    try {
        $spec = Get-Content $job.FullName -Raw | ConvertFrom-Json
    } catch {
        Log "job $name : UNREADABLE ($($_.Exception.Message)) -- discarding"
        Remove-Item $job.FullName -Force -ErrorAction SilentlyContinue
        continue
    }
    Remove-Item $job.FullName -Force -ErrorAction SilentlyContinue

    $snap = if ($null -ne $spec.Snaplen)   { [int]$spec.Snaplen }   else { 4096 }
    $secs = if ($null -ne $spec.MaxSeconds){ [int]$spec.MaxSeconds }else { 300 }
    $buf  = if ($null -ne $spec.BufferLen) { [int]$spec.BufferLen } else { 0 }

    Log "job $name : START prefix=$($spec.OutPrefix) snaplen=$snap max=${secs}s buflen=$buf"
    Set-Content -Path (Join-Path $jobs "$name.started") -Encoding utf8 `
                -Value (Get-Date -Format 'yyyy-MM-dd HH:mm:ss.fff')

    try {
        & $capture -OutPrefix $spec.OutPrefix -Snaplen $snap -MaxSeconds $secs `
                   -FlagDir $out -BufferLen $buf 2>&1 | ForEach-Object { Log "  | $_" }
    } catch {
        Log "job $name : ERROR $($_.Exception.Message)"
    }

    $sizes = @()
    foreach ($f in @("$($spec.OutPrefix)-usbpcap1.pcap", "$($spec.OutPrefix)-usbpcap2.pcap")) {
        $sz = if (Test-Path $f) { (Get-Item $f).Length } else { -1 }
        $sizes += "$f = $sz"
        Log "job $name : $f -> $sz bytes"
    }

    Set-Content -Path (Join-Path $jobs "$name.finished") -Encoding utf8 -Value @"
finished=$(Get-Date -Format 'yyyy-MM-dd HH:mm:ss.fff')
$($sizes -join "`n")
"@
    Log "job $name : DONE"
}

Log "=== capture-runner exiting (quit.flag) ==="
Remove-Item (Join-Path $jobs 'runner.ready') -Force -ErrorAction SilentlyContinue
Remove-Item $quit -Force -ErrorAction SilentlyContinue
