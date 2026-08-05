# Read and set a display's scaling percentage from the command line.
#
# The runbook needs the dock screens at 100%: the test content is 2560x1440 to land one
# video pixel on one panel pixel, and any other scale factor makes the compositor resample
# it, so every decoded pixel on the wire becomes a blend of two and the bit-depth and
# gamut probes stop meaning anything. player.html checks for this (devicePixelRatio 1,
# "1:1 mapping yes") but cannot fix it.
#
#   .\dpiscale.ps1 -List
#   .\dpiscale.ps1 -Display \\.\DISPLAY29 -Percent 100
#
# This uses the *undocumented* DISPLAYCONFIG_DEVICE_INFO_GET/SET_DPI_SCALE (-3 / -4)
# device-info types, which is what Settings itself drives. They are stable across Win10
# and Win11 but are not in the SDK, so treat a failure here as "do it in Settings by
# hand", not as something to force.
#
# The scale is expressed *relative to the scale Windows recommends* for that display, not
# as an absolute percentage -- hence the index arithmetic below.
param(
    [switch]$List,
    [string]$Display,
    [int]$Percent = 0
)

$ErrorActionPreference = 'Stop'

if (-not ('DpiCcd' -as [type])) {
Add-Type @"
using System;
using System.Runtime.InteropServices;

public static class DpiCcd {
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
    public struct MODE_INFO {
        public uint infoType; public uint id; public LUID adapterId;
    }

    [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)]
    public struct SOURCE_DEVICE_NAME {
        public DEVICE_INFO_HEADER header;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst=32)] public string viewGdiDeviceName;
    }

    // DISPLAYCONFIG_DEVICE_INFO_GET_DPI_SCALE = -3. All three values are indices
    // *relative to the recommended scale*, so cur == 0 means "running at recommended".
    [StructLayout(LayoutKind.Sequential)]
    public struct DPI_SCALE_GET {
        public DEVICE_INFO_HEADER header;
        public int minScaleRel; public int curScaleRel; public int maxScaleRel;
    }

    // DISPLAYCONFIG_DEVICE_INFO_SET_DPI_SCALE = -4.
    [StructLayout(LayoutKind.Sequential)]
    public struct DPI_SCALE_SET {
        public DEVICE_INFO_HEADER header;
        public int scaleRel;
    }

    [DllImport("user32.dll")]
    public static extern int GetDisplayConfigBufferSizes(uint flags, out uint numPath, out uint numMode);
    [DllImport("user32.dll")]
    public static extern int QueryDisplayConfig(uint flags, ref uint numPath,
        [Out] PATH_INFO[] paths, ref uint numMode, [Out] MODE_INFO[] modes, IntPtr topologyId);
    [DllImport("user32.dll")]
    public static extern int DisplayConfigGetDeviceInfo(ref SOURCE_DEVICE_NAME req);
    [DllImport("user32.dll")]
    public static extern int DisplayConfigGetDeviceInfo(ref DPI_SCALE_GET req);
    [DllImport("user32.dll")]
    public static extern int DisplayConfigSetDeviceInfo(ref DPI_SCALE_SET req);

    public static int SizeOfT(Type t) { return Marshal.SizeOf(t); }
}
"@
}

$QDC_ONLY_ACTIVE_PATHS = 0x2
# The scale factors Windows will actually accept, in order. curScaleRel indexes into this.
$DPI_VALS = @(100,125,150,175,200,225,250,300,350,400,450,500)

# Same value-type-copy trap as hdr.ps1: assign the header whole, never field by field.
function New-Header([int]$type, [int]$size, $adapterId, [uint32]$id) {
    $h = New-Object DpiCcd+DEVICE_INFO_HEADER
    $h.type = $type
    $h.size = $size
    $h.adapterId = $adapterId
    $h.id = $id
    return $h
}

function Get-Sources {
    $np = 0; $nm = 0
    $r = [DpiCcd]::GetDisplayConfigBufferSizes($QDC_ONLY_ACTIVE_PATHS, [ref]$np, [ref]$nm)
    if ($r -ne 0) { throw "GetDisplayConfigBufferSizes failed: $r" }
    $paths = New-Object 'DpiCcd+PATH_INFO[]' $np
    $modes = New-Object 'DpiCcd+MODE_INFO[]' $nm
    $r = [DpiCcd]::QueryDisplayConfig($QDC_ONLY_ACTIVE_PATHS, [ref]$np, $paths, [ref]$nm, $modes, [IntPtr]::Zero)
    if ($r -ne 0) { throw "QueryDisplayConfig failed: $r" }

    $out = @()
    for ($i = 0; $i -lt $np; $i++) {
        $p = $paths[$i]

        $src = New-Object DpiCcd+SOURCE_DEVICE_NAME
        $src.header = New-Header 1 ([DpiCcd]::SizeOfT([DpiCcd+SOURCE_DEVICE_NAME])) $p.sourceInfo.adapterId $p.sourceInfo.id
        $gdi = if ([DpiCcd]::DisplayConfigGetDeviceInfo([ref]$src) -eq 0) { $src.viewGdiDeviceName } else { '?' }

        $g = New-Object DpiCcd+DPI_SCALE_GET
        $g.header = New-Header (-3) ([DpiCcd]::SizeOfT([DpiCcd+DPI_SCALE_GET])) $p.sourceInfo.adapterId $p.sourceInfo.id
        $ok = ([DpiCcd]::DisplayConfigGetDeviceInfo([ref]$g) -eq 0)

        $cur = $null; $recIdx = $null; $curPct = $null; $avail = @()
        if ($ok) {
            # cur/min/max are offsets from the recommended index. Recover the absolute
            # index of the recommended scale by anchoring on 0 == recommended.
            $recIdx = -$g.minScaleRel
            $curIdx = $recIdx + $g.curScaleRel
            if ($curIdx -ge 0 -and $curIdx -lt $DPI_VALS.Count) { $curPct = $DPI_VALS[$curIdx] }
            for ($k = $g.minScaleRel; $k -le $g.maxScaleRel; $k++) {
                $idx = $recIdx + $k
                if ($idx -ge 0 -and $idx -lt $DPI_VALS.Count) { $avail += $DPI_VALS[$idx] }
            }
            $cur = $g.curScaleRel
        }

        $out += [pscustomobject]@{
            Index      = $i
            Gdi        = $gdi
            Ok         = $ok
            CurRel     = $cur
            MinRel     = $(if ($ok) { $g.minScaleRel } else { $null })
            MaxRel     = $(if ($ok) { $g.maxScaleRel } else { $null })
            RecIdx     = $recIdx
            CurPercent = $curPct
            RecPercent = $(if ($ok -and $recIdx -lt $DPI_VALS.Count) { $DPI_VALS[$recIdx] } else { $null })
            Available  = $avail
            AdapterLow = $p.sourceInfo.adapterId.LowPart
            AdapterHigh= $p.sourceInfo.adapterId.HighPart
            SourceId   = $p.sourceInfo.id
        }
    }
    return $out
}

if ($List -or (-not $Display)) {
    Get-Sources | ForEach-Object {
        Write-Output ''
        Write-Output ("[{0}] {1}" -f $_.Index, $_.Gdi)
        if ($_.Ok) {
            Write-Output ("     scale {0}%  (recommended {1}%)  rel cur={2} min={3} max={4}" -f
                          $_.CurPercent, $_.RecPercent, $_.CurRel, $_.MinRel, $_.MaxRel)
            Write-Output ("     available: {0}" -f (($_.Available | ForEach-Object { "$_%" }) -join ' '))
        } else {
            Write-Output "     (DPI scale query not supported on this path)"
        }
    }
    Write-Output ''
    return
}

if ($Percent -le 0) { throw "give -Percent (e.g. -Percent 100), or -List" }

$all = Get-Sources
$m = @($all | Where-Object { $_.Gdi -eq $Display })
if ($m.Count -eq 0) { $m = @($all | Where-Object { $_.Gdi -like "*$Display*" }) }
if ($m.Count -ne 1) { throw "'$Display' matched $($m.Count) sources. Run -List." }
$e = $m[0]
if (-not $e.Ok) { throw "cannot read DPI scale for $($e.Gdi) -- set it in Settings by hand" }

$targetIdx = [array]::IndexOf($DPI_VALS, $Percent)
if ($targetIdx -lt 0) { throw "$Percent% is not one of: $($DPI_VALS -join ', ')" }

$rel = $targetIdx - $e.RecIdx
if ($rel -lt $e.MinRel -or $rel -gt $e.MaxRel) {
    throw "$Percent% is outside what this display allows ($($e.Available -join '%, ')%)"
}

if ($e.CurPercent -eq $Percent) {
    Write-Output ("{0}  {1} already at {2}%" -f (Get-Date -Format 'yyyy-MM-dd HH:mm:ss.fff'), $e.Gdi, $Percent)
    return
}

$luid = New-Object DpiCcd+LUID
$luid.LowPart  = $e.AdapterLow
$luid.HighPart = $e.AdapterHigh

$req = New-Object DpiCcd+DPI_SCALE_SET
$req.header = New-Header (-4) ([DpiCcd]::SizeOfT([DpiCcd+DPI_SCALE_SET])) $luid $e.SourceId
$req.scaleRel = $rel

$r = [DpiCcd]::DisplayConfigSetDeviceInfo([ref]$req)
Start-Sleep -Milliseconds 800

$after = @(Get-Sources | Where-Object { $_.Gdi -eq $e.Gdi })[0]
Write-Output ("{0}  {1}  {2}% -> {3}%  rc={4}  now {5}%" -f
    (Get-Date -Format 'yyyy-MM-dd HH:mm:ss.fff'), $e.Gdi, $e.CurPercent, $Percent, $r, $after.CurPercent)

if ($after.CurPercent -ne $Percent) {
    Write-Output "!! scale did not take -- set it in Settings > System > Display > Scale"
    exit 1
}
