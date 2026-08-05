# Read the "SDR content brightness" slider, and log every change with a timestamp.
#
# Runbook phase 7.2 moves that slider from minimum to maximum and back while a capture runs.
# The slider has no public setter, so the move has to be done by hand -- but it does have a
# getter, DISPLAYCONFIG_GET_SDR_WHITE_LEVEL (device info type 12), and that is enough to turn
# "the operator moved a slider at some point" into a list of exact instants. Without this the
# capture is a few minutes of traffic with no idea which part is which.
#
# SDRWhiteLevel is in 1/1000ths of a scale where 1000 == 80 nits, so nits = level / 1000 * 80.
#
#   .\sdr-whitelevel.ps1 -Display \\.\DISPLAY29 -Seconds 180 -LogPath out\cap15.phaselog.txt
param(
    [string]$Display = '\\.\DISPLAY29',
    [int]$Seconds = 180,
    [int]$PollMs = 200,
    [string]$LogPath = ''
)

$ErrorActionPreference = 'Stop'

if (-not ('SdrCcd' -as [type])) {
Add-Type @"
using System;
using System.Runtime.InteropServices;

public static class SdrCcd {
    [StructLayout(LayoutKind.Sequential)]
    public struct LUID { public uint LowPart; public int HighPart; }

    [StructLayout(LayoutKind.Sequential)]
    public struct DEVICE_INFO_HEADER {
        public int type; public uint size; public LUID adapterId; public uint id;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct PATH_SOURCE_INFO {
        public LUID adapterId; public uint id; public uint modeInfoIdx; public uint statusFlags;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct RATIONAL { public uint Numerator; public uint Denominator; }

    [StructLayout(LayoutKind.Sequential)]
    public struct PATH_TARGET_INFO {
        public LUID adapterId; public uint id; public uint modeInfoIdx;
        public uint outputTechnology; public uint rotation; public uint scaling;
        public RATIONAL refreshRate; public uint scanLineOrdering;
        public int targetAvailable; public uint statusFlags;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct PATH_INFO {
        public PATH_SOURCE_INFO sourceInfo; public PATH_TARGET_INFO targetInfo; public uint flags;
    }

    [StructLayout(LayoutKind.Sequential, Size=64)]
    public struct MODE_INFO { public uint infoType; public uint id; public LUID adapterId; }

    [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)]
    public struct SOURCE_DEVICE_NAME {
        public DEVICE_INFO_HEADER header;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst=32)] public string viewGdiDeviceName;
    }

    // DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL = 11
    // (9 = GET_ADVANCED_COLOR_INFO, 10 = SET_ADVANCED_COLOR_STATE, 11 = this.
    //  Asking for 12 returns success with a zero level, which looks like a slider at
    //  minimum rather than like the error it is.)
    [StructLayout(LayoutKind.Sequential)]
    public struct SDR_WHITE_LEVEL {
        public DEVICE_INFO_HEADER header;
        public uint SDRWhiteLevel;
    }

    // DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO = 9
    [StructLayout(LayoutKind.Sequential)]
    public struct ADVANCED_COLOR_INFO {
        public DEVICE_INFO_HEADER header;
        public uint value; public uint colorEncoding; public uint bitsPerColorChannel;
    }

    [DllImport("user32.dll")]
    public static extern int GetDisplayConfigBufferSizes(uint flags, out uint numPath, out uint numMode);
    [DllImport("user32.dll")]
    public static extern int QueryDisplayConfig(uint flags, ref uint numPath,
        [Out] PATH_INFO[] paths, ref uint numMode, [Out] MODE_INFO[] modes, IntPtr topologyId);
    [DllImport("user32.dll")]
    public static extern int DisplayConfigGetDeviceInfo(ref SOURCE_DEVICE_NAME req);
    [DllImport("user32.dll")]
    public static extern int DisplayConfigGetDeviceInfo(ref SDR_WHITE_LEVEL req);
    [DllImport("user32.dll")]
    public static extern int DisplayConfigGetDeviceInfo(ref ADVANCED_COLOR_INFO req);
    public static int SizeOfT(Type t) { return Marshal.SizeOf(t); }
}
"@
}

function New-Header([int]$type, [int]$size, $adapterId, [uint32]$id) {
    $h = New-Object SdrCcd+DEVICE_INFO_HEADER
    $h.type = $type; $h.size = $size; $h.adapterId = $adapterId; $h.id = $id
    return $h
}

function Get-Target([string]$gdi) {
    $np = 0; $nm = 0
    [void][SdrCcd]::GetDisplayConfigBufferSizes(2, [ref]$np, [ref]$nm)
    $paths = New-Object 'SdrCcd+PATH_INFO[]' $np
    $modes = New-Object 'SdrCcd+MODE_INFO[]' $nm
    [void][SdrCcd]::QueryDisplayConfig(2, [ref]$np, $paths, [ref]$nm, $modes, [IntPtr]::Zero)
    for ($i = 0; $i -lt $np; $i++) {
        $p = $paths[$i]
        $src = New-Object SdrCcd+SOURCE_DEVICE_NAME
        $src.header = New-Header 1 ([SdrCcd]::SizeOfT([SdrCcd+SOURCE_DEVICE_NAME])) $p.sourceInfo.adapterId $p.sourceInfo.id
        if ([SdrCcd]::DisplayConfigGetDeviceInfo([ref]$src) -ne 0) { continue }
        if ($src.viewGdiDeviceName -eq $gdi) { return $p.targetInfo }
    }
    throw "no active path for $gdi"
}

function Read-State($tgt) {
    $w = New-Object SdrCcd+SDR_WHITE_LEVEL
    $w.header = New-Header 11 ([SdrCcd]::SizeOfT([SdrCcd+SDR_WHITE_LEVEL])) $tgt.adapterId $tgt.id
    $okw = ([SdrCcd]::DisplayConfigGetDeviceInfo([ref]$w) -eq 0)

    $a = New-Object SdrCcd+ADVANCED_COLOR_INFO
    $a.header = New-Header 9 ([SdrCcd]::SizeOfT([SdrCcd+ADVANCED_COLOR_INFO])) $tgt.adapterId $tgt.id
    $oka = ([SdrCcd]::DisplayConfigGetDeviceInfo([ref]$a) -eq 0)

    return [pscustomobject]@{
        Level = $(if ($okw) { $w.SDRWhiteLevel } else { $null })
        Nits  = $(if ($okw) { [math]::Round($w.SDRWhiteLevel / 1000.0 * 80.0, 1) } else { $null })
        Hdr   = $(if ($oka) { [bool]($a.value -band 2) } else { $null })
        Bpc   = $(if ($oka) { $a.bitsPerColorChannel } else { $null })
    }
}

function Log([string]$t) {
    $line = "{0}  {1}" -f (Get-Date -Format 'yyyy-MM-dd HH:mm:ss.fff'), $t
    Write-Host $line
    if ($LogPath) { Add-Content -Path $LogPath -Value $line -Encoding utf8 }
}

$tgt = Get-Target $Display
$s = Read-State $tgt
Log ("SDRWL   watching $Display -- start level=$($s.Level) (~$($s.Nits) nits) hdr=$($s.Hdr) bpc=$($s.Bpc)")

$last = $s.Level
$min = $s.Level; $max = $s.Level
$changes = 0
$deadline = (Get-Date).AddSeconds($Seconds)

while ((Get-Date) -lt $deadline) {
    Start-Sleep -Milliseconds $PollMs
    $s = Read-State $tgt
    if ($null -ne $s.Level -and $s.Level -ne $last) {
        $changes++
        if ($s.Level -lt $min) { $min = $s.Level }
        if ($s.Level -gt $max) { $max = $s.Level }
        Log ("SDRWL   level {0,6} -> {1,6}  (~{2} nits)  bpc={3}" -f $last, $s.Level, $s.Nits, $s.Bpc)
        $last = $s.Level
    }
}

Log ("SDRWL   done -- $changes changes, range $min..$max (~$([math]::Round($min/1000.0*80,1))..$([math]::Round($max/1000.0*80,1)) nits)")
if ($changes -eq 0) {
    Log 'SDRWL   !! the slider never moved -- phase 7.2 did not actually happen'
    exit 2
}
