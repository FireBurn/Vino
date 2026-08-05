# Enumerate every mode a dock display supports, and optionally walk through all of them.
#
# The dock display is identified as "non-primary with the largest X origin" (the rightmost
# monitor) rather than by device name, because DisplayLink displays are RENAMED on every mode
# change (\\.\DISPLAY9 -> \\.\DISPLAY37 -> \\.\DISPLAY85 ...). The rightmost monitor stays
# rightmost across resolution changes, so this stays stable for the whole sweep.
param(
    [switch]$ListOnly,
    [double]$DwellSeconds = 3,
    [string]$LogFile = '',
    [int]$RestoreW = 2560,
    [int]$RestoreH = 1440,
    [int]$RestoreHz = 60
)

Add-Type -AssemblyName System.Windows.Forms

if (-not ('Disp4' -as [type])) {
Add-Type @"
using System;
using System.Runtime.InteropServices;

public class Disp4 {
    [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)]
    public struct DEVMODE {
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst=32)] public string dmDeviceName;
        public short dmSpecVersion, dmDriverVersion, dmSize, dmDriverExtra;
        public int dmFields;
        public int dmPositionX, dmPositionY;
        public int dmDisplayOrientation, dmDisplayFixedOutput;
        public short dmColor, dmDuplex, dmYResolution, dmTTOption, dmCollate;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst=32)] public string dmFormName;
        public short dmLogPixels;
        public int dmBitsPerPel, dmPelsWidth, dmPelsHeight;
        public int dmDisplayFlags, dmDisplayFrequency;
        public int dmICMMethod, dmICMIntent, dmMediaType, dmDitherType;
        public int dmReserved1, dmReserved2, dmPanningWidth, dmPanningHeight;
    }
    [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    public static extern bool EnumDisplaySettingsW(string dev, int mode, ref DEVMODE dm);
    [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    public static extern int ChangeDisplaySettingsExW(string dev, ref DEVMODE dm, IntPtr hwnd,
                                                      uint flags, IntPtr param);
    public static int DevmodeSize() { return Marshal.SizeOf(typeof(DEVMODE)); }
}
"@
}

$ENUM_CURRENT = -1
$DM_PELSWIDTH = 0x80000; $DM_PELSHEIGHT = 0x100000
$DM_DISPLAYFREQUENCY = 0x400000; $DM_BITSPERPEL = 0x40000
$CDS_UPDATEREGISTRY = 0x01

function Get-DockDevice {
    $s = [System.Windows.Forms.Screen]::AllScreens |
         Where-Object { -not $_.Primary } |
         Sort-Object { $_.Bounds.X } -Descending |
         Select-Object -First 1
    if (-not $s) { throw "no non-primary display found" }
    return $s.DeviceName
}

function Set-Mode([string]$dev, [int]$w, [int]$h, [int]$hz) {
    $dm = New-Object Disp4+DEVMODE
    $dm.dmSize = [int16][Disp4]::DevmodeSize()
    if (-not [Disp4]::EnumDisplaySettingsW($dev, $ENUM_CURRENT, [ref]$dm)) { return 'READ-FAILED' }
    $dm.dmPelsWidth = $w; $dm.dmPelsHeight = $h; $dm.dmDisplayFrequency = $hz; $dm.dmBitsPerPel = 32
    $dm.dmFields = $DM_PELSWIDTH -bor $DM_PELSHEIGHT -bor $DM_DISPLAYFREQUENCY -bor $DM_BITSPERPEL
    $r = [Disp4]::ChangeDisplaySettingsExW($dev, [ref]$dm, [IntPtr]::Zero, $CDS_UPDATEREGISTRY, [IntPtr]::Zero)
    switch ($r) {
        0  { 'OK' } 1 { 'RESTART-NEEDED' } -1 { 'FAILED' } -2 { 'BADMODE' }
        -3 { 'NOTUPDATED' } -4 { 'BADFLAGS' } -5 { 'BADPARAM' } default { "rc=$r" }
    }
}

$dev = Get-DockDevice

# Enumerate all modes
$modes = @()
$i = 0
while ($true) {
    $dm = New-Object Disp4+DEVMODE
    $dm.dmSize = [int16][Disp4]::DevmodeSize()
    if (-not [Disp4]::EnumDisplaySettingsW($dev, $i, [ref]$dm)) { break }
    if ($dm.dmBitsPerPel -eq 32 -and $dm.dmDisplayFrequency -gt 1) {
        $modes += [pscustomobject]@{
            W = $dm.dmPelsWidth; H = $dm.dmPelsHeight; Hz = $dm.dmDisplayFrequency
        }
    }
    $i++
}
$modes = $modes | Sort-Object W, H, Hz -Unique

Write-Output "dock display : $dev"
Write-Output "modes found  : $($modes.Count)  (32bpp, deduped)"
Write-Output ""

if ($ListOnly) {
    $modes | Group-Object { "$($_.W)x$($_.H)" } | ForEach-Object {
        $hzs = ($_.Group | ForEach-Object { $_.Hz } | Sort-Object -Unique) -join ', '
        Write-Output ("  {0,-12} Hz: {1}" -f $_.Name, $hzs)
    }
    return
}

# --- sweep ---
$log = @()
$log += "mode sweep - dock display $dev - started $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss.fff')"
$log += "dwell ${DwellSeconds}s per mode, $($modes.Count) modes"
$log += ""
$log += "HH:MM:SS.fff   WxH @ Hz          result"

foreach ($m in $modes) {
    $dev = Get-DockDevice          # re-resolve: the name changes on every mode change
    $t = Get-Date -Format 'HH:mm:ss.fff'
    $r = Set-Mode $dev $m.W $m.H $m.Hz
    $line = ("{0}   {1,-18} {2}" -f $t, "$($m.W)x$($m.H) @ $($m.Hz)", $r)
    $log += $line
    Write-Output $line
    Start-Sleep -Milliseconds ([int]($DwellSeconds * 1000))
}

# restore
$dev = Get-DockDevice
$t = Get-Date -Format 'HH:mm:ss.fff'
$r = Set-Mode $dev $RestoreW $RestoreH $RestoreHz
$line = ("{0}   RESTORE {1,-10} {2}" -f $t, "${RestoreW}x${RestoreH} @ $RestoreHz", $r)
$log += $line
Write-Output $line
$log += "finished $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss.fff')"

if ($LogFile) { $log | Out-File -Encoding utf8 $LogFile; Write-Output "log -> $LogFile" }
