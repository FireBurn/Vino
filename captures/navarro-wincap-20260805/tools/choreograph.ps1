# Drive a capture phase end to end, with every action timestamped.
#
# The runbook's phases are sequences of "do this, wait N seconds, do that", where what makes
# the capture readable afterwards is knowing exactly when each step happened. Doing that by
# hand means a stopwatch, a keyboard and a notepad, and the previous session shows how that
# goes: cap6 and cap7 were both labelled HDR-on/HDR-off and both played the same SDR clip.
#
# So the page is driven over the DevTools protocol (tools\cdp.ps1) and HDR over the display
# config API (tools\hdr.ps1), and every step is written to a log with millisecond stamps.
# The capture itself runs in the elevated runner (tools\capture-runner.ps1).
#
#   .\choreograph.ps1 -Phase h1
#   .\choreograph.ps1 -Phase h2 -Head '\\.\DISPLAY30' -Other '\\.\DISPLAY29'
#
# Preconditions, all checked before anything is captured:
#   * Edge is up on the target head with player.html and --remote-debugging-port=9222
#   * the elevated runner is alive
#   * the player reports devicePixelRatio 1 and 1:1 mapping
param(
    [Parameter(Mandatory=$true)]
    [ValidateSet('h1','h2','probes','axes','bandwidth')]
    [string]$Phase,

    [string]$Head  = '\\.\DISPLAY29',   # the head the player is on
    [string]$Other = '\\.\DISPLAY30',   # the other dock head
    [string]$Root  = 'C:\Users\Mike\navarro-wincap',
    [string]$OutPrefix,
    [int]$Snaplen  = -1,
    [int]$MaxSeconds = 0,
    [switch]$DryRun                      # walk the steps with no capture, to rehearse timing
)

$ErrorActionPreference = 'Stop'

$out    = Join-Path $Root 'out'
$jobs   = Join-Path $out  'jobs'
$hdrPs  = Join-Path $Root 'tools\hdr.ps1'
$cdpPs  = Join-Path $Root 'tools\cdp.ps1'
$psexe  = (Get-Command powershell.exe).Source

# Phase defaults: the full-payload ones are the codec captures, the 4096 ones are about
# control messages. -BufferLen is always the maximum; the 1 MB default drops nearly
# everything from a lit dock.
$DEFAULTS = @{
    h1        = @{ Prefix = 'cap9-hdr-ab';           Snaplen = 0;    Max = 480 }
    h2        = @{ Prefix = 'cap10-hdr-ab-ep0a';     Snaplen = 0;    Max = 300 }
    probes    = @{ Prefix = 'cap11-metadata-probes'; Snaplen = 4096; Max = 220 }
    axes      = @{ Prefix = 'cap12-axes';            Snaplen = 4096; Max = 240 }
    bandwidth = @{ Prefix = 'cap13-bandwidth';       Snaplen = 4096; Max = 240 }
}
$d = $DEFAULTS[$Phase]
if (-not $OutPrefix)      { $OutPrefix = Join-Path $out $d.Prefix }
if ($Snaplen -lt 0)       { $Snaplen = $d.Snaplen }
if ($MaxSeconds -le 0)    { $MaxSeconds = $d.Max }

$logPath = "$OutPrefix.phaselog.txt"
$script:steps = @()

function Log([string]$text) {
    $line = "{0}  {1}" -f (Get-Date -Format 'yyyy-MM-dd HH:mm:ss.fff'), $text
    Write-Host $line
    Add-Content -Path $logPath -Value $line -Encoding utf8
    $script:steps += $line
}

function Cdp([string]$js, [switch]$Gesture) {
    $a = @('-NoProfile','-ExecutionPolicy','Bypass','-File',$cdpPs,'-Js',$js)
    if ($Gesture) { $a += '-UserGesture' }
    return (& $psexe @a 2>&1 | Out-String).Trim()
}

function Hdr([string]$disp, [bool]$on, [string]$tag) {
    $a = @('-NoProfile','-ExecutionPolicy','Bypass','-File',$hdrPs,'-Display',$disp,
           $(if ($on) { '-On' } else { '-Off' }),'-Tag',$tag)
    $r = (& $psexe @a 2>&1 | Out-String).Trim()
    Log "HDR     $r"
}

# A "wait" is a step in its own right: the idle stretches are what let the analysis tell a
# quiet wire from a busy one, so they get logged with their intended length.
function Idle([int]$seconds, [string]$label) {
    Log ("IDLE    {0,-22} {1} s -- video paused on black" -f $label, $seconds)
    [void](Cdp "(()=>{const v=document.getElementById('v');v.pause();v.currentTime=0.05;return 'paused';})()")
    Start-Sleep -Seconds $seconds
}

function Play([int]$sourceIndex, [string]$label, [int]$seconds) {
    $r = Cdp "(()=>{load(SOURCES[$sourceIndex]);return SOURCES[$sourceIndex].src;})()" -Gesture
    Log ("PLAY    {0,-22} {1} s  src={2}" -f $label, $seconds, $r)
    Start-Sleep -Seconds $seconds
}

function Check-Player([string]$when) {
    $j = Cdp @'
(()=>{const v=document.getElementById('v');const dpr=window.devicePixelRatio||1;
const r=v.getBoundingClientRect();
return JSON.stringify({hi:window.matchMedia('(dynamic-range: high)').matches,
dpr:dpr,one:!(v.videoWidth&&Math.abs(r.width*dpr-v.videoWidth)>1),
depth:screen.colorDepth,src:v.currentSrc.replace(/^.*\//,''),err:v.error?v.error.code:null,
drop:v.getVideoPlaybackQuality?v.getVideoPlaybackQuality().droppedVideoFrames:null});})()
'@
    Log "PLAYER  $when  $j"
    return ($j | ConvertFrom-Json)
}

# --------------------------------------------------------------- preflight ----

Log "=== phase $Phase ==="
Log "head=$Head other=$Other prefix=$OutPrefix snaplen=$Snaplen max=${MaxSeconds}s dryrun=$DryRun"

if (-not (Test-Path (Join-Path $jobs 'runner.ready'))) {
    throw "elevated capture runner is not up -- start tools\capture-runner.ps1 with -Verb RunAs"
}

# Hide the player's own panels: they are static, but they are pixels that are not in the
# reference images, and a decoded strip that disagrees with ref\decoded\ for a reason as
# dull as an info panel is a trap worth not setting.
[void](Cdp "(()=>{for(const id of ['info','log','hint'])document.getElementById(id).classList.add('hidden');return 'panels hidden';})()")

$pre = Check-Player 'preflight'
if ($pre.dpr -ne 1)  { throw "devicePixelRatio is $($pre.dpr), not 1 -- set that display to 100% scaling" }
if (-not $pre.one)   { throw "picture is not 1:1 -- it is being resampled; fix before capturing" }

# ----------------------------------------------------------------- capture ----

$name = [IO.Path]::GetFileName($OutPrefix)
if (-not $DryRun) {
    Remove-Item "$jobs\$name.finished" -ErrorAction SilentlyContinue
    Remove-Item "$jobs\$name.started"  -ErrorAction SilentlyContinue
    $spec = @{ OutPrefix = $OutPrefix; Snaplen = $Snaplen; MaxSeconds = $MaxSeconds
               BufferLen = 134217728 } | ConvertTo-Json
    Set-Content -Path "$jobs\$name.job" -Value $spec -Encoding utf8
    $t0 = Get-Date
    while (-not (Test-Path "$jobs\$name.started") -and ((Get-Date) - $t0).TotalSeconds -lt 30) {
        Start-Sleep -Milliseconds 200
    }
    if (-not (Test-Path "$jobs\$name.started")) { throw "runner never started the job" }
    Log "CAPTURE started ($name)"
    Start-Sleep -Seconds 3      # let USBPcap settle before the first interesting thing
}

# ------------------------------------------------------------------- steps ----

switch ($Phase) {

  # The A/B the last session could not do: the same 14 pictures, same mode, same connector,
  # HDR on and then HDR off. Anything that differs is the HDR mode or the content's range.
  { $_ -in @('h1','h2') } {
        Hdr $Other $false "$Phase : other head SDR throughout"
        Hdr $Head  $false "$Phase : start from SDR"
        Idle 15 'idle (pre)'

        Hdr $Head $true "$Phase step 2: HDR ON"
        Start-Sleep -Seconds 3
        $c = Check-Player 'after HDR ON'
        if (-not $c.hi) { Log "!! WARNING dynamic-range is NOT high with HDR on -- capture is suspect" }

        Play 0 'hdr-pattern' 92
        if ($Phase -eq 'h1') { Play 4 'hdr-motion' 32 }
        Idle 15 'settle'

        Hdr $Head $false "$Phase step 6: HDR OFF"
        Start-Sleep -Seconds 3
        [void](Check-Player 'after HDR OFF')

        Play 2 'sdr-pattern' 92
        if ($Phase -eq 'h1') { Play 6 'sdr-motion' 32 }
        Idle 15 'idle (post)'
  }

  # Seven clips, byte-identical pixels, different HDR10 static metadata. The page runs the
  # sequence itself so the order and the gaps cannot be got wrong.
  'probes' {
        Hdr $Other $false 'probes: other head SDR'
        Hdr $Head  $true  'probes: HDR ON for the whole sequence'
        Start-Sleep -Seconds 3
        $c = Check-Player 'before probes'
        if (-not $c.hi) { Log "!! WARNING dynamic-range is NOT high -- probes will not mean much" }

        # HEVC: if the mp4s cannot decode, this phase cannot run at all.
        $probe = Cdp @'
(()=>{const t=document.createElement('video');
return JSON.stringify({hevc:t.canPlayType('video/mp4; codecs="hvc1.2.4.L153.B0"'),
hevc8:t.canPlayType('video/mp4; codecs="hvc1.1.6.L93.B0"')});})()
'@
        Log "HEVC    support: $probe"

        Idle 15 'idle (pre)'
        [void](Cdp "(()=>{runProbes(0);return 'probe sequence started';})()" -Gesture)
        Log 'PROBES  sequence started (7 clips, 6.5 s each, 15 s black between)'
        Start-Sleep -Seconds 155
        $c = Check-Player 'after probes'
        if ($c.err) { Log "!! probe clips reported decode error $($c.err) -- HEVC likely missing" }
        Idle 15 'idle (post)'
  }

  # The non-content axes: does the set-mode message change when HDR is toggled with nothing
  # playing at all? That isolates the mode change from any content.
  'axes' {
        Hdr $Other $false 'axes: other head SDR'
        Hdr $Head  $false 'axes: start SDR'
        Idle 20 'static desktop, SDR'

        Hdr $Head $true 'axes 1: HDR ON, static desktop'
        Idle 20 'static desktop, HDR on'

        Hdr $Head $false 'axes 1: HDR OFF, static desktop'
        Idle 20 'static desktop, SDR again'

        Hdr $Head $true 'axes 1: HDR ON again (repeatability)'
        Idle 20 'static desktop, HDR on again'
        Hdr $Head $false 'axes 1: HDR OFF again'
        Idle 15 'idle (post)'
        Log 'NOTE    SDR-content-brightness slider (7.2) is NOT scripted -- no public API; do by hand'
  }

  # Two heads at once, then the depth-vs-bandwidth question at 120 Hz.
  'bandwidth' {
        Hdr $Other $false 'bw: both heads SDR to start'
        Hdr $Head  $false 'bw: both heads SDR to start'
        Idle 15 'both SDR'

        Hdr $Head $true 'bw 1a: HDR on capture head only'
        Play 0 'hdr-pattern, one head HDR' 40

        Hdr $Other $true 'bw 1b: HDR on BOTH heads'
        Start-Sleep -Seconds 40
        Log 'STATE   both heads HDR, pattern still playing'

        Hdr $Head $false 'bw 1c: HDR on the OTHER head only'
        Start-Sleep -Seconds 40
        Idle 15 'settle'
        Hdr $Other $false 'bw: back to both SDR'
        Log 'NOTE    120 Hz step (7b.2) is run separately -- it changes the mode, see NOTES.md'
  }
}

# ------------------------------------------------------------------ finish ----

if (-not $DryRun) {
    Set-Content -Path (Join-Path $out 'stop.flag') -Value 'x' -Encoding utf8
    Log 'CAPTURE stop.flag written'
    $t0 = Get-Date
    while (-not (Test-Path "$jobs\$name.finished") -and ((Get-Date) - $t0).TotalSeconds -lt 90) {
        Start-Sleep -Milliseconds 500
    }
    if (Test-Path "$jobs\$name.finished") {
        Log 'CAPTURE finished'
        Get-Content "$jobs\$name.finished" | ForEach-Object { Log "  $_" }
    } else {
        Log '!! runner did not report finished within 90 s'
    }
    Remove-Item (Join-Path $out 'stop.flag') -ErrorAction SilentlyContinue
}

Log "=== phase $Phase complete ==="
Write-Host ''
Write-Host "phase log: $logPath"
