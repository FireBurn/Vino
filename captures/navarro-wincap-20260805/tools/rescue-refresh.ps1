# Get the dock out of a mode it cannot sustain.
#
# 2560x1440 @ 180 Hz knocks the DL7400 into a reconnect loop: the whole hub
# re-enumerates every few seconds, so the dock screens never settle and the
# desktop is unusable. Windows persists the mode per monitor, so it comes back
# after a reboot -- you have to change it, not wait it out.
#
# The awkward part is that you cannot reliably drive a display that is busy
# re-enumerating. So the order here is: cut output to the dock first, change
# the *stored* mode while nothing is fighting you, then bring it back.
#
#   .\rescue-refresh.ps1 -List                     what is attached, and its modes
#   .\rescue-refresh.ps1                           the rescue: everything to 60 Hz
#   .\rescue-refresh.ps1 -Hz 120                   ... to some other refresh
#   .\rescue-refresh.ps1 -Hz 60 -Width 2560 -Height 1440
#   .\rescue-refresh.ps1 -NoDisplaySwitch          skip the internal-only step
#
# Run it from the LAPTOP'S OWN SCREEN. If the dock screens are looping you will
# not be able to see a window that is on them.
param(
    [int]$Hz = 60,
    [int]$Width = 0,             # 0 = keep whatever width is stored
    [int]$Height = 0,
    [switch]$List,
    [switch]$NoDisplaySwitch,
    [int]$TimeoutSeconds = 90,
    [switch]$AllDisplays,         # default is non-primary only
    # Restrict the change to one display, e.g. -Device '\\.\DISPLAY29'. The rescue case
    # wants every dock head at once, but phase 7b.2 changes the refresh of the head under
    # test only, so that the other head stays a fixed control.
    [string]$Device = ''
)

$ErrorActionPreference = 'Stop'

if (-not ('Rescue' -as [type])) {
Add-Type @"
using System;
using System.Runtime.InteropServices;

public class Rescue {
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

    [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)]
    public struct DISPLAY_DEVICE {
        public int cb;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst=32)]  public string DeviceName;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst=128)] public string DeviceString;
        public int StateFlags;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst=128)] public string DeviceID;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst=128)] public string DeviceKey;
    }

    [DllImport("user32.dll", CharSet=CharSet.Unicode)]
    public static extern bool EnumDisplayDevicesW(string dev, uint num,
                                                  ref DISPLAY_DEVICE dd, uint flags);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)]
    public static extern bool EnumDisplaySettingsW(string dev, int mode, ref DEVMODE dm);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)]
    public static extern int ChangeDisplaySettingsExW(string dev, ref DEVMODE dm,
                                                      IntPtr hwnd, uint flags, IntPtr p);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)]
    public static extern int ChangeDisplaySettingsExW(string dev, IntPtr dm,
                                                      IntPtr hwnd, uint flags, IntPtr p);
    public static int Size() { return Marshal.SizeOf(typeof(DEVMODE)); }
    public static int DdSize() { return Marshal.SizeOf(typeof(DISPLAY_DEVICE)); }
}
"@
}

$ENUM_CURRENT  = -1
$ENUM_REGISTRY = -2
$DM_PELSWIDTH        = 0x80000
$DM_PELSHEIGHT       = 0x100000
$DM_DISPLAYFREQUENCY = 0x400000
$DM_BITSPERPEL       = 0x40000
$CDS_UPDATEREGISTRY  = 0x01
$CDS_NORESET         = 0x10000000

$ATTACHED_TO_DESKTOP = 0x01
$PRIMARY_DEVICE      = 0x04

function Get-Devices {
    # EnumDisplayDevices, unlike Screen.AllScreens, also lists displays that are
    # currently detached -- which is exactly the state we put the dock into
    # before changing its stored mode.
    $out = @()
    $i = 0
    while ($true) {
        $dd = New-Object Rescue+DISPLAY_DEVICE
        $dd.cb = [Rescue]::DdSize()
        # [NullString]::Value, not $null: PowerShell converts $null to "" when it
        # marshals to a [string] P/Invoke parameter, and EnumDisplayDevices("")
        # fails outright -- so this loop returned nothing at all.
        if (-not [Rescue]::EnumDisplayDevicesW([NullString]::Value, $i, [ref]$dd, 0)) { break }
        $out += [pscustomobject]@{
            Name     = $dd.DeviceName
            Desc     = $dd.DeviceString
            Attached = [bool]($dd.StateFlags -band $ATTACHED_TO_DESKTOP)
            Primary  = [bool]($dd.StateFlags -band $PRIMARY_DEVICE)
            Id       = $dd.DeviceID
        }
        $i++
    }
    return $out
}

function Get-Mode([string]$dev, [int]$which) {
    $dm = New-Object Rescue+DEVMODE
    $dm.dmSize = [int16][Rescue]::Size()
    if ([Rescue]::EnumDisplaySettingsW($dev, $which, [ref]$dm)) { return $dm }
    return $null
}

function Get-AllModes([string]$dev) {
    $modes = @()
    $i = 0
    while ($true) {
        $dm = New-Object Rescue+DEVMODE
        $dm.dmSize = [int16][Rescue]::Size()
        if (-not [Rescue]::EnumDisplaySettingsW($dev, $i, [ref]$dm)) { break }
        $modes += [pscustomobject]@{
            W = $dm.dmPelsWidth; H = $dm.dmPelsHeight
            Hz = $dm.dmDisplayFrequency; Bpp = $dm.dmBitsPerPel
        }
        $i++
    }
    return $modes
}

function Result-Text([int]$r) {
    switch ($r) {
        0  { 'SUCCESSFUL' }
        1  { 'RESTART NEEDED' }
        -1 { 'FAILED' }
        -2 { 'BADMODE (that refresh is not offered at that resolution)' }
        -3 { 'NOTUPDATED (could not write the registry)' }
        -4 { 'BADFLAGS' }
        -5 { 'BADPARAM' }
        default { "unknown ($r)" }
    }
}

# ------------------------------------------------------------------ list ----

if ($List) {
    foreach ($d in Get-Devices) {
        $cur = Get-Mode $d.Name $ENUM_CURRENT
        $reg = Get-Mode $d.Name $ENUM_REGISTRY
        Write-Output ''
        Write-Output "$($d.Name)  $($d.Desc)"
        Write-Output ("  attached=$($d.Attached) primary=$($d.Primary)")
        if ($cur) { Write-Output ("  current  : {0}x{1} @ {2} Hz {3} bpp" -f $cur.dmPelsWidth, $cur.dmPelsHeight, $cur.dmDisplayFrequency, $cur.dmBitsPerPel) }
        if ($reg) { Write-Output ("  stored   : {0}x{1} @ {2} Hz {3} bpp" -f $reg.dmPelsWidth, $reg.dmPelsHeight, $reg.dmDisplayFrequency, $reg.dmBitsPerPel) }
        $modes = Get-AllModes $d.Name
        if ($modes.Count) {
            $byRes = $modes | Group-Object { "$($_.W)x$($_.H)" }
            foreach ($g in $byRes) {
                $hzs = ($g.Group | Select-Object -ExpandProperty Hz | Sort-Object -Unique) -join ' '
                Write-Output ("  modes    : {0,-12} Hz: {1}" -f $g.Name, $hzs)
            }
        } else {
            Write-Output "  modes    : (none enumerable -- display is detached or busy)"
        }
    }
    return
}

# ---------------------------------------------------------------- rescue ----

Write-Output "=== rescue-refresh: target $Hz Hz ==="
Write-Output "before:"
foreach ($d in Get-Devices) {
    $cur = Get-Mode $d.Name $ENUM_CURRENT
    if ($cur) { Write-Output ("  {0,-16} {1}x{2} @ {3} Hz  attached={4} primary={5}" -f $d.Name, $cur.dmPelsWidth, $cur.dmPelsHeight, $cur.dmDisplayFrequency, $d.Attached, $d.Primary) }
}

if (-not $NoDisplaySwitch) {
    # Cut output to everything except the built-in panel. This stops the dock
    # being driven at all, which is what breaks the reconnect loop; it also
    # means nothing is contending for the display while we rewrite the mode.
    Write-Output ''
    Write-Output "-> DisplaySwitch.exe /internal  (dock goes dark; this is expected)"
    Start-Process -FilePath "$env:WINDIR\System32\DisplaySwitch.exe" -ArgumentList '/internal' -Wait
    Start-Sleep -Seconds 6
}

$deadline = (Get-Date).AddSeconds($TimeoutSeconds)
$fixed = @{}
$attempt = 0

while ((Get-Date) -lt $deadline) {
    $attempt++
    # Skip the phantom heads the GPUs expose but that have no monitor on them:
    # they have no stored mode, can never succeed, and retrying them burns the
    # whole timeout while the two displays we care about are already done.
    $targets = Get-Devices |
        Where-Object { $AllDisplays -or (-not $_.Primary) } |
        Where-Object { (-not $Device) -or ($_.Name -eq $Device) } |
        Where-Object {
            $m = Get-Mode $_.Name $ENUM_REGISTRY
            if (-not $m) { $m = Get-Mode $_.Name $ENUM_CURRENT }
            $m -and $m.dmPelsWidth -gt 0
        }
    $pending = @($targets | Where-Object { -not $fixed.ContainsKey($_.Name) })
    if ($pending.Count -eq 0 -and $attempt -gt 1) { break }

    foreach ($d in $pending) {
        # Prefer the stored mode as the base: while detached there is no
        # "current" mode to read, but the stored one is what a re-attach uses.
        $dm = Get-Mode $d.Name $ENUM_REGISTRY
        if (-not $dm) { $dm = Get-Mode $d.Name $ENUM_CURRENT }
        if (-not $dm) { continue }

        if ($Width -gt 0)  { $dm.dmPelsWidth  = $Width }
        if ($Height -gt 0) { $dm.dmPelsHeight = $Height }
        $was = "$($dm.dmPelsWidth)x$($dm.dmPelsHeight) @ $($dm.dmDisplayFrequency)"
        $dm.dmDisplayFrequency = $Hz
        if ($dm.dmBitsPerPel -le 0) { $dm.dmBitsPerPel = 32 }
        $dm.dmFields = $DM_PELSWIDTH -bor $DM_PELSHEIGHT -bor $DM_DISPLAYFREQUENCY -bor $DM_BITSPERPEL

        # NORESET writes the registry without applying; the single apply below
        # then commits every display at once, which is far less likely to leave
        # the dock half-configured than one reset per display.
        $r = [Rescue]::ChangeDisplaySettingsExW($d.Name, [ref]$dm, [IntPtr]::Zero,
                                                ($CDS_UPDATEREGISTRY -bor $CDS_NORESET), [IntPtr]::Zero)
        $t = Get-Date -Format 'HH:mm:ss'
        Write-Output ("  {0} attempt {1,-3} {2,-16} {3} -> {4} Hz : {5}" -f $t, $attempt, $d.Name, $was, $Hz, (Result-Text $r))
        if ($r -eq 0) { $fixed[$d.Name] = $true }
    }
    if (@(($targets | Where-Object { -not $fixed.ContainsKey($_.Name) })).Count -eq 0) { break }
    Start-Sleep -Seconds 2
}

Write-Output ''
Write-Output "-> committing"
$r = [Rescue]::ChangeDisplaySettingsExW([NullString]::Value, [IntPtr]::Zero, [IntPtr]::Zero, 0, [IntPtr]::Zero)
Write-Output ("   apply: {0}" -f (Result-Text $r))

if (-not $NoDisplaySwitch) {
    Write-Output "-> DisplaySwitch.exe /extend"
    Start-Process -FilePath "$env:WINDIR\System32\DisplaySwitch.exe" -ArgumentList '/extend' -Wait
    Start-Sleep -Seconds 10
}

Write-Output ''
Write-Output "after:"
$bad = @()
foreach ($d in Get-Devices) {
    $cur = Get-Mode $d.Name $ENUM_CURRENT
    if ($cur) {
        Write-Output ("  {0,-16} {1}x{2} @ {3} Hz  attached={4} primary={5}" -f $d.Name, $cur.dmPelsWidth, $cur.dmPelsHeight, $cur.dmDisplayFrequency, $d.Attached, $d.Primary)
        if ((-not $d.Primary) -and $cur.dmDisplayFrequency -gt $Hz) { $bad += $d.Name }
    }
}

if ($bad.Count) {
    Write-Output ''
    Write-Output "!! still above $Hz Hz: $($bad -join ', ')"
    Write-Output "   Try, in order:"
    Write-Output "     1. run this again -- the dock may have been mid-reconnect"
    Write-Output "     2. .\rescue-refresh.ps1 -Hz $Hz -Width 1920 -Height 1080"
    Write-Output "        (a lower resolution needs less link bandwidth to come up at all)"
    Write-Output "     3. unplug the dock, run it again so the stored mode is written"
    Write-Output "        while nothing is attached, then plug the dock back in"
    Write-Output "     4. Settings > System > Display > Advanced display, set the"
    Write-Output "        refresh rate by hand on each dock screen"
} else {
    Write-Output ''
    Write-Output "OK -- no non-primary display above $Hz Hz."
}
