# Read and set Windows' per-display HDR ("advanced colour") state from the command line.
#
# The runbook needs HDR toggled at known wall-clock instants, several times, with the
# result logged. Doing that through Settings > System > Display > HDR gives you a click
# whose time you have to write down by hand and a state you cannot read back. This does
# it through the same API Settings uses -- DisplayConfigSetDeviceInfo with
# DISPLAYCONFIG_SET_ADVANCED_COLOR_STATE -- and prints an ISO timestamp for the change.
#
#   .\hdr.ps1 -List                        every attached path: HDR support/state, bit depth
#   .\hdr.ps1 -Display \\.\DISPLAY27 -On
#   .\hdr.ps1 -Display \\.\DISPLAY27 -Off
#   .\hdr.ps1 -Display \\.\DISPLAY27 -On -Tag "H1 step 2 HDR ON"    # tag goes in the log line
#
# -Display accepts the GDI name (\\.\DISPLAY27), or a substring of the monitor's friendly
# name, or a path index from -List.
param(
    [switch]$List,
    [string]$Display,
    [switch]$On,
    [switch]$Off,
    [string]$Tag = ''
)

$ErrorActionPreference = 'Stop'

if (-not ('CCD' -as [type])) {
Add-Type @"
using System;
using System.Runtime.InteropServices;

public static class CCD {
    [StructLayout(LayoutKind.Sequential)]
    public struct LUID { public uint LowPart; public int HighPart; }

    [StructLayout(LayoutKind.Sequential)]
    public struct DEVICE_INFO_HEADER {
        public uint type; public uint size; public LUID adapterId; public uint id;
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

    // 64 bytes: infoType + id + adapterId + the largest union member (targetMode, 48).
    [StructLayout(LayoutKind.Sequential, Size=64)]
    public struct MODE_INFO {
        public uint infoType; public uint id; public LUID adapterId;
    }

    [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)]
    public struct SOURCE_DEVICE_NAME {
        public DEVICE_INFO_HEADER header;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst=32)] public string viewGdiDeviceName;
    }

    [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)]
    public struct TARGET_DEVICE_NAME {
        public DEVICE_INFO_HEADER header;
        public uint flags;
        public uint outputTechnology;
        public ushort edidManufactureId;
        public ushort edidProductCodeId;
        public uint connectorInstance;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst=64)]  public string monitorFriendlyDeviceName;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst=128)] public string monitorDevicePath;
    }

    // DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO, type 9.
    [StructLayout(LayoutKind.Sequential)]
    public struct GET_ADVANCED_COLOR_INFO {
        public DEVICE_INFO_HEADER header;
        public uint value;              // bit0 supported, bit1 enabled, bit2 wideColorEnforced,
                                        // bit3 forceDisabled
        public uint colorEncoding;
        public uint bitsPerColorChannel;
    }

    // DISPLAYCONFIG_SET_ADVANCED_COLOR_STATE, type 10.
    [StructLayout(LayoutKind.Sequential)]
    public struct SET_ADVANCED_COLOR_STATE {
        public DEVICE_INFO_HEADER header;
        public uint value;              // bit0 enableAdvancedColor
    }

    [DllImport("user32.dll")]
    public static extern int GetDisplayConfigBufferSizes(uint flags, out uint numPath, out uint numMode);
    [DllImport("user32.dll")]
    public static extern int QueryDisplayConfig(uint flags, ref uint numPath,
        [Out] PATH_INFO[] paths, ref uint numMode, [Out] MODE_INFO[] modes, IntPtr topologyId);
    [DllImport("user32.dll")]
    public static extern int DisplayConfigGetDeviceInfo(ref SOURCE_DEVICE_NAME req);
    [DllImport("user32.dll")]
    public static extern int DisplayConfigGetDeviceInfo(ref TARGET_DEVICE_NAME req);
    [DllImport("user32.dll")]
    public static extern int DisplayConfigGetDeviceInfo(ref GET_ADVANCED_COLOR_INFO req);
    [DllImport("user32.dll")]
    public static extern int DisplayConfigSetDeviceInfo(ref SET_ADVANCED_COLOR_STATE req);

    public static int SizeOfT(Type t) { return Marshal.SizeOf(t); }
}
"@
}

$QDC_ONLY_ACTIVE_PATHS = 0x2
$ENCODINGS = @{ 0='RGB'; 1='YCbCr444'; 2='YCbCr422'; 3='YCbCr420'; 4='Intensity' }
$OUTPUT_TECH = @{
    0='HD15'; 4='DVI'; 5='HDMI'; 6='LVDS'; 8='SDI';
    9='DisplayPort external'; 10='DisplayPort embedded';
    11='UDI external'; 12='UDI embedded'; 15='Internal'
}

# PowerShell hands back a *copy* when you read a value-type field, so
# `$req.header.type = 1` mutates a temporary and the real struct stays zeroed --
# which makes every DisplayConfigGetDeviceInfo call fail with a null type/size.
# Build the header as a whole and assign it in one go.
function New-Header([uint32]$type, [int]$size, $adapterId, [uint32]$id) {
    $h = New-Object CCD+DEVICE_INFO_HEADER
    $h.type = $type
    $h.size = $size
    $h.adapterId = $adapterId
    $h.id = $id
    return $h
}

function Get-Paths {
    $np = 0; $nm = 0
    $r = [CCD]::GetDisplayConfigBufferSizes($QDC_ONLY_ACTIVE_PATHS, [ref]$np, [ref]$nm)
    if ($r -ne 0) { throw "GetDisplayConfigBufferSizes failed: $r" }
    $paths = New-Object 'CCD+PATH_INFO[]' $np
    $modes = New-Object 'CCD+MODE_INFO[]' $nm
    $r = [CCD]::QueryDisplayConfig($QDC_ONLY_ACTIVE_PATHS, [ref]$np, $paths, [ref]$nm, $modes, [IntPtr]::Zero)
    if ($r -ne 0) { throw "QueryDisplayConfig failed: $r" }

    $out = @()
    for ($i = 0; $i -lt $np; $i++) {
        $p = $paths[$i]

        $src = New-Object CCD+SOURCE_DEVICE_NAME
        $src.header = New-Header 1 ([CCD]::SizeOfT([CCD+SOURCE_DEVICE_NAME])) $p.sourceInfo.adapterId $p.sourceInfo.id
        $gdi = if ([CCD]::DisplayConfigGetDeviceInfo([ref]$src) -eq 0) { $src.viewGdiDeviceName } else { '?' }

        $tgt = New-Object CCD+TARGET_DEVICE_NAME
        $tgt.header = New-Header 2 ([CCD]::SizeOfT([CCD+TARGET_DEVICE_NAME])) $p.targetInfo.adapterId $p.targetInfo.id
        # NB: not $tech -- PowerShell variable names are case-insensitive, so a local
        # $tech and the $OUTPUT_TECH table would collide if that table were named $TECH.
        $friendly = '?'; $devpath = ''; $conn = 0; $techId = [int64]$p.targetInfo.outputTechnology
        if ([CCD]::DisplayConfigGetDeviceInfo([ref]$tgt) -eq 0) {
            $friendly = $tgt.monitorFriendlyDeviceName
            $devpath  = $tgt.monitorDevicePath
            $conn     = $tgt.connectorInstance
        }

        $aci = New-Object CCD+GET_ADVANCED_COLOR_INFO
        $aci.header = New-Header 9 ([CCD]::SizeOfT([CCD+GET_ADVANCED_COLOR_INFO])) $p.targetInfo.adapterId $p.targetInfo.id
        $sup=$null; $en=$null; $wide=$null; $forced=$null; $enc=$null; $bpc=$null
        if ([CCD]::DisplayConfigGetDeviceInfo([ref]$aci) -eq 0) {
            $sup    = [bool]($aci.value -band 1)
            $en     = [bool]($aci.value -band 2)
            $wide   = [bool]($aci.value -band 4)
            $forced = [bool]($aci.value -band 8)
            $enc    = $ENCODINGS[[int]$aci.colorEncoding]
            $bpc    = $aci.bitsPerColorChannel
        }

        $hz = if ($p.targetInfo.refreshRate.Denominator) {
            [math]::Round($p.targetInfo.refreshRate.Numerator / $p.targetInfo.refreshRate.Denominator, 2)
        } else { 0 }

        $out += [pscustomobject]@{
            Index        = $i
            Gdi          = $gdi
            Friendly     = $friendly
            Tech         = $(if ($OUTPUT_TECH.ContainsKey($techId)) { $OUTPUT_TECH[$techId] } else { "tech0x{0:x}" -f $techId })
            ConnInstance = $conn
            Hz           = $hz
            HdrSupported = $sup
            HdrEnabled   = $en
            WideEnforced = $wide
            ForceDisabled= $forced
            Encoding     = $enc
            BitsPerCh    = $bpc
            AdapterLow   = $p.targetInfo.adapterId.LowPart
            AdapterHigh  = $p.targetInfo.adapterId.HighPart
            TargetId     = $p.targetInfo.id
            DevicePath   = $devpath
        }
    }
    return $out
}

function Resolve-Path-Entry([string]$want) {
    $all = Get-Paths
    $m = @($all | Where-Object { $_.Gdi -eq $want })
    if ($m.Count -eq 0) { $m = @($all | Where-Object { $_.Index -eq ($want -as [int]) }) }
    if ($m.Count -eq 0) { $m = @($all | Where-Object { $_.Gdi -like "*$want*" -or $_.Friendly -like "*$want*" }) }
    if ($m.Count -eq 0) { throw "no active display path matches '$want'. Run -List." }
    if ($m.Count -gt 1) { throw "'$want' matches $($m.Count) paths: $(($m | ForEach-Object { $_.Gdi }) -join ', ')" }
    return $m[0]
}

# ------------------------------------------------------------------ list ----

if ($List -or (-not $Display)) {
    Get-Paths | ForEach-Object {
        Write-Output ''
        Write-Output ("[{0}] {1,-14} {2}" -f $_.Index, $_.Gdi, $_.Friendly)
        Write-Output ("     {0}, connector instance {1}, {2} Hz" -f $_.Tech, $_.ConnInstance, $_.Hz)
        Write-Output ("     HDR supported={0} enabled={1} wideColorEnforced={2} forceDisabled={3}" -f
                      $_.HdrSupported, $_.HdrEnabled, $_.WideEnforced, $_.ForceDisabled)
        Write-Output ("     encoding={0} bitsPerColourChannel={1}" -f $_.Encoding, $_.BitsPerCh)
    }
    Write-Output ''
    return
}

# ------------------------------------------------------------------- set ----

if (-not ($On -or $Off)) { throw "give -On or -Off (or -List)" }
if ($On -and $Off)       { throw "-On and -Off are mutually exclusive" }

$e = Resolve-Path-Entry $Display
$want = [bool]$On

if (-not $e.HdrSupported) {
    Write-Output "!! $($e.Gdi) ($($e.Friendly)) reports HDR NOT SUPPORTED -- not touching it."
    Write-Output "   That is itself a finding; record it in out\NOTES.md."
    exit 2
}

$before = $e.HdrEnabled
$luid = New-Object CCD+LUID
$luid.LowPart  = $e.AdapterLow
$luid.HighPart = $e.AdapterHigh

$req = New-Object CCD+SET_ADVANCED_COLOR_STATE
$req.header = New-Header 10 ([CCD]::SizeOfT([CCD+SET_ADVANCED_COLOR_STATE])) $luid $e.TargetId
$req.value = if ($want) { 1 } else { 0 }

$t0 = Get-Date
$r = [CCD]::DisplayConfigSetDeviceInfo([ref]$req)
$t1 = Get-Date

Start-Sleep -Milliseconds 700
$after = (Resolve-Path-Entry $Display)

$stamp = $t0.ToString('yyyy-MM-dd HH:mm:ss.fff')
$state = if ($want) { 'ON' } else { 'OFF' }
$okstr = if ($r -eq 0) { 'ok' } else { "FAILED (rc=$r)" }
Write-Output ("{0}  HDR {1,-3} {2,-14} {3,-24} {4}  enabled {5} -> {6}  bpc {7}  enc {8}  [{9} ms]{10}" -f
    $stamp, $state, $e.Gdi, $e.Friendly, $okstr, $before, $after.HdrEnabled,
    $after.BitsPerCh, $after.Encoding, [int]($t1 - $t0).TotalMilliseconds,
    $(if ($Tag) { "  $Tag" } else { '' }))

if ($r -ne 0) { exit 1 }
if ($after.HdrEnabled -ne $want) {
    Write-Output "!! state did not change as asked -- check the monitor is still attached"
    exit 3
}
