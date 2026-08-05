# Evaluate JavaScript in the running player page over the DevTools protocol.
#
# Two jobs, both of which the runbook otherwise leaves to a human eye and a keyboard:
#
#   * Verify the preconditions it insists on -- "dynamic-range high", devicePixelRatio 1,
#     "1:1 mapping yes". Read them, don't squint at them; a capture taken through a window
#     Edge thinks is SDR is a wasted run and it is not obvious afterwards.
#   * Drive the choreography by calling the page's own functions, so each step happens at a
#     time we know to the millisecond instead of when a keystroke happened to land.
#
# Needs Edge started with --remote-debugging-port=9222 (see the launch in NOTES.md).
#
#   .\cdp.ps1 -Js "1+1"
#   .\cdp.ps1 -Check                       # the runbook's preconditions, as an object
#   .\cdp.ps1 -Js "load(SOURCES[0])" -UserGesture
param(
    [string]$Js,
    # Anything with quotes in it gets mangled by the time PowerShell has built a child
    # process command line, so non-trivial snippets go in a file instead of an argument.
    [string]$JsFile,
    [switch]$Check,
    [string]$TargetTitle = 'DL7400 HDR capture player',
    [int]$Port = 9222,
    [switch]$UserGesture,
    [switch]$Raw,
    [int]$TimeoutSec = 20
)

$ErrorActionPreference = 'Stop'

function Get-Target {
    $list = (Invoke-WebRequest "http://127.0.0.1:$Port/json/list" -UseBasicParsing -TimeoutSec 10).Content | ConvertFrom-Json
    $t = @($list | Where-Object { $_.type -eq 'page' -and $_.title -eq $TargetTitle })
    if ($t.Count -eq 0) { $t = @($list | Where-Object { $_.type -eq 'page' -and $_.url -like '*player.html*' }) }
    if ($t.Count -eq 0) { throw "no player page on the debugger port. Is Edge still up with --remote-debugging-port=$Port?" }
    return $t[0]
}

function Invoke-Cdp([string]$wsUrl, [string]$expression, [bool]$gesture) {
    $ws = New-Object System.Net.WebSockets.ClientWebSocket
    $cts = New-Object System.Threading.CancellationTokenSource
    $cts.CancelAfter([TimeSpan]::FromSeconds($TimeoutSec))
    try {
        $ws.ConnectAsync([Uri]$wsUrl, $cts.Token).GetAwaiter().GetResult()

        $msg = @{
            id     = 1
            method = 'Runtime.evaluate'
            params = @{
                expression    = $expression
                returnByValue = $true
                awaitPromise  = $true
                userGesture   = $gesture
            }
        } | ConvertTo-Json -Depth 8 -Compress

        $bytes = [Text.Encoding]::UTF8.GetBytes($msg)
        $seg = New-Object System.ArraySegment[byte] -ArgumentList @(,$bytes)
        $ws.SendAsync($seg, [System.Net.WebSockets.WebSocketMessageType]::Text, $true, $cts.Token).GetAwaiter().GetResult()

        # Responses and events share the socket; keep reading until our id comes back.
        $sb = New-Object Text.StringBuilder
        while ($true) {
            $buf = New-Object byte[] 65536
            $rseg = New-Object System.ArraySegment[byte] -ArgumentList @(,$buf)
            [void]$sb.Clear()
            do {
                $res = $ws.ReceiveAsync($rseg, $cts.Token).GetAwaiter().GetResult()
                [void]$sb.Append([Text.Encoding]::UTF8.GetString($buf, 0, $res.Count))
            } while (-not $res.EndOfMessage)

            $obj = $sb.ToString() | ConvertFrom-Json
            if ($obj.PSObject.Properties.Name -contains 'id' -and $obj.id -eq 1) { return $obj }
        }
    } finally {
        try { $ws.Dispose() } catch {}
        $cts.Dispose()
    }
}

# The exact set of things the runbook says to confirm before every HDR capture,
# plus enough playback state to tell a decoding video from a stalled one.
$CHECK_JS = @'
(() => {
  const v = document.getElementById('v');
  const dpr = window.devicePixelRatio || 1;
  const r = v.getBoundingClientRect();
  const q = v.getVideoPlaybackQuality ? v.getVideoPlaybackQuality() : null;
  return {
    dynamicRangeHigh:      window.matchMedia('(dynamic-range: high)').matches,
    videoDynamicRangeHigh: window.matchMedia('(video-dynamic-range: high)').matches,
    devicePixelRatio: dpr,
    oneToOne: !(v.videoWidth && Math.abs(r.width * dpr - v.videoWidth) > 1),
    cssSize: r.width + 'x' + r.height,
    deviceSize: (r.width * dpr) + 'x' + (r.height * dpr),
    videoSize: v.videoWidth + 'x' + v.videoHeight,
    screen: screen.width + 'x' + screen.height,
    colorDepth: screen.colorDepth,
    src: v.currentSrc.replace(/^.*\//, ''),
    readyState: v.readyState,
    paused: v.paused,
    currentTime: +v.currentTime.toFixed(2),
    duration: +(v.duration || 0).toFixed(2),
    error: v.error ? v.error.code : null,
    totalFrames: q ? q.totalVideoFrames : null,
    droppedFrames: q ? q.droppedVideoFrames : null,
    fullscreen: !!document.fullscreenElement,
    outerSize: window.outerWidth + 'x' + window.outerHeight,
    screenPos: window.screenX + ',' + window.screenY
  };
})()
'@

if ($Check)   { $Js = $CHECK_JS }
if ($JsFile)  { $Js = Get-Content -Raw -Path $JsFile }
if (-not $Js) { throw "give -Js <expression>, -JsFile <path> or -Check" }

$t = Get-Target
$resp = Invoke-Cdp $t.webSocketDebuggerUrl $Js ([bool]$UserGesture)

if ($Raw) { $resp | ConvertTo-Json -Depth 12; return }

if ($resp.result.exceptionDetails) {
    Write-Output "JS EXCEPTION: $($resp.result.exceptionDetails.text)"
    Write-Output ($resp.result.exceptionDetails.exception.description)
    exit 1
}
$val = $resp.result.result.value
if ($null -eq $val) { Write-Output "(undefined)" } else { $val }
