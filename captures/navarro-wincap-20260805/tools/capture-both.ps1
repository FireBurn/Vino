# Elevated capture harness — captures ALL USBPcap root hubs at once.
# The dock re-enumerates on replug and can land on either hub, so we do not gamble on one.
# Runs until a stop.flag sentinel appears (or MaxSeconds), then stops every child gracefully.
param(
    [Parameter(Mandatory=$true)][string]$OutPrefix,   # e.g. C:\...\out\cap1  -> cap1-usbpcap1.pcap
    [Parameter(Mandatory=$true)][int]$Snaplen,
    [Parameter(Mandatory=$true)][int]$MaxSeconds,
    [Parameter(Mandatory=$true)][string]$FlagDir,
    [string[]]$Devices = @('\\.\USBPcap1', '\\.\USBPcap2'),
    # USBPcap's kernel-mode ring buffer. The 1 MB default is far too small for full-payload
    # capture of a lit dock (~280 KB per URB) and silently drops almost everything.
    # Valid range 4096..134217728.
    [int]$BufferLen = 0
)

$stop = Join-Path $FlagDir 'stop.flag'
$done = Join-Path $FlagDir 'done.flag'
$log  = Join-Path $FlagDir 'capture.log'

Remove-Item $stop -ErrorAction SilentlyContinue
Remove-Item $done -ErrorAction SilentlyContinue

"=== capture-both ===" | Out-File -Encoding utf8 $log
"snaplen  : $Snaplen"  | Add-Content $log
"started  : $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss.fff')" | Add-Content $log

$procs = @()
foreach ($dev in $Devices) {
    $tag  = ($dev -replace '.*\\', '').ToLower()          # \\.\USBPcap1 -> usbpcap1
    $file = "$OutPrefix-$tag.pcap"
    Remove-Item $file -ErrorAction SilentlyContinue
    # NOTE: USBPcapCMD does NOT accept -s 0 as "unlimited" (unlike tcpdump) -- it exits
    # immediately. Omit -s entirely for the default, or pass a large explicit value.
    $a = @('-d', $dev, '-o', $file)
    if ($Snaplen -gt 0)   { $a += @('-s', "$Snaplen") }
    if ($BufferLen -gt 0) { $a += @('-b', "$BufferLen") }
    $a += @('-A', '--inject-descriptors')

    $errFile = "$OutPrefix-$tag.stderr.txt"
    $p = Start-Process -FilePath "C:\Program Files\USBPcap\USBPcapCMD.exe" `
                       -ArgumentList $a -PassThru -WindowStyle Minimized `
                       -RedirectStandardError $errFile
    "  $dev -> $file  pid=$($p.Id)  args=[$($a -join ' ')]" | Add-Content $log
    $procs += [pscustomobject]@{ Proc = $p; File = $file; Dev = $dev }
}

$deadline = (Get-Date).AddSeconds($MaxSeconds)
while (-not (Test-Path $stop) -and (Get-Date) -lt $deadline) {
    if (($procs | Where-Object { -not $_.Proc.HasExited }).Count -eq 0) { break }
    Start-Sleep -Milliseconds 300
}

$why = if (Test-Path $stop) { 'sentinel' } else { 'timeout/exit' }
"stopping : $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss.fff')  reason=$why" | Add-Content $log

foreach ($e in $procs) {
    if (-not $e.Proc.HasExited) { & taskkill /PID $e.Proc.Id 2>&1 | Out-Null }
}
Start-Sleep -Seconds 2
foreach ($e in $procs) {
    if (-not $e.Proc.HasExited) { Stop-Process -Id $e.Proc.Id -Force -ErrorAction SilentlyContinue }
}
Start-Sleep -Seconds 1

"exited   : $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss.fff')" | Add-Content $log
foreach ($e in $procs) {
    $sz = if (Test-Path $e.File) { (Get-Item $e.File).Length } else { 'MISSING' }
    "  $($e.File) : $sz bytes" | Add-Content $log
}
"done" | Out-File -Encoding utf8 $done
