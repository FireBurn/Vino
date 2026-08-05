# Query and set display modes.
# Device names come from System.Windows.Forms.Screen (reliable) rather than EnumDisplayDevices;
# mode get/set uses EnumDisplaySettingsW / ChangeDisplaySettingsExW.
#
#   .\displaymode.ps1                                  -> list current modes
#   .\displaymode.ps1 -SetHz 180 -Only '\\.\DISPLAY9','\\.\DISPLAY10'
#   .\displaymode.ps1 -SetHz 60  -Only '\\.\DISPLAY9','\\.\DISPLAY10'
param(
    [int]$SetHz = 0,
    [string[]]$Only = @(),
    # DisplayLink displays are renamed (\\.\DISPLAY9 -> \\.\DISPLAY37) whenever the mode changes,
    # so targeting them by name is unreliable. -NonPrimary selects every non-primary display,
    # which on this host is exactly the two dock monitors.
    [switch]$NonPrimary
)

Add-Type -AssemblyName System.Windows.Forms

if (-not ('Disp3' -as [type])) {
Add-Type @"
using System;
using System.Runtime.InteropServices;

public class Disp3 {
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

$ENUM_CURRENT        = -1
$DM_PELSWIDTH        = 0x80000
$DM_PELSHEIGHT       = 0x100000
$DM_DISPLAYFREQUENCY = 0x400000
$DM_BITSPERPEL       = 0x40000
$CDS_UPDATEREGISTRY  = 0x01

function Get-Mode([string]$dev) {
    $dm = New-Object Disp3+DEVMODE
    $dm.dmSize = [int16][Disp3]::DevmodeSize()
    if ([Disp3]::EnumDisplaySettingsW($dev, $ENUM_CURRENT, [ref]$dm)) { return $dm }
    return $null
}

function Show-All {
    $rows = @()
    foreach ($s in [System.Windows.Forms.Screen]::AllScreens) {
        $dm = Get-Mode $s.DeviceName
        if ($dm) {
            $rows += [pscustomobject]@{
                Device  = $s.DeviceName
                Primary = $s.Primary
                Mode    = "$($dm.dmPelsWidth)x$($dm.dmPelsHeight)"
                Hz      = $dm.dmDisplayFrequency
                Bpp     = $dm.dmBitsPerPel
                Origin  = "$($dm.dmPositionX),$($dm.dmPositionY)"
            }
        } else {
            $rows += [pscustomobject]@{ Device=$s.DeviceName; Primary=$s.Primary; Mode='(read failed)'; Hz=''; Bpp=''; Origin='' }
        }
    }
    $rows | Format-Table -AutoSize
}

if ($SetHz -le 0) { Show-All; return }

$targets = if ($Only.Count) {
    $Only
} elseif ($NonPrimary) {
    @([System.Windows.Forms.Screen]::AllScreens | Where-Object { -not $_.Primary } | ForEach-Object { $_.DeviceName })
} else {
    [System.Windows.Forms.Screen]::AllScreens.DeviceName
}

foreach ($dev in $targets) {
    $dm = Get-Mode $dev
    if (-not $dm) { Write-Output "$dev : could not read current mode"; continue }
    $was = $dm.dmDisplayFrequency
    $dm.dmDisplayFrequency = $SetHz
    $dm.dmFields = $DM_PELSWIDTH -bor $DM_PELSHEIGHT -bor $DM_DISPLAYFREQUENCY -bor $DM_BITSPERPEL
    $t = Get-Date -Format 'HH:mm:ss.fff'
    $r = [Disp3]::ChangeDisplaySettingsExW($dev, [ref]$dm, [IntPtr]::Zero, $CDS_UPDATEREGISTRY, [IntPtr]::Zero)
    $meaning = switch ($r) {
        0  { 'SUCCESSFUL' }
        1  { 'RESTART NEEDED' }
        -1 { 'FAILED' }
        -2 { 'BADMODE - refresh not supported at this resolution' }
        -3 { 'NOTUPDATED' }
        -4 { 'BADFLAGS' }
        -5 { 'BADPARAM' }
        default { "unknown ($r)" }
    }
    Write-Output "$t  $dev : $was Hz -> $SetHz Hz = $meaning"
}

Write-Output ""
Write-Output "--- modes now ---"
Show-All
