# Display a known test pattern fullscreen on one monitor, and save the pixel-exact source as PNG.
# The saved PNG is the ground truth the captured video payload should decode to.
#
#   .\test-pattern.ps1 -Device '\\.\DISPLAY9' -Seconds 90 -SavePng C:\...\out\screen-ref.png
param(
    [Parameter(Mandatory=$true)][string]$Device,
    [int]$Seconds = 90,
    [string]$SavePng = ''
)

Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

$screen = [System.Windows.Forms.Screen]::AllScreens | Where-Object { $_.DeviceName -eq $Device }
if (-not $screen) { throw "no such display: $Device" }
$b = $screen.Bounds
$W = $b.Width; $H = $b.Height

$bmp = New-Object System.Drawing.Bitmap($W, $H, [System.Drawing.Imaging.PixelFormat]::Format32bppRgb)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::None
$g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::NearestNeighbor
$g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::Half

$g.Clear([System.Drawing.Color]::Black)

# --- Band 1: saturated primaries, large flat areas with hard vertical edges ---
$bars = @(
    [System.Drawing.Color]::FromArgb(255,0,0),
    [System.Drawing.Color]::FromArgb(0,255,0),
    [System.Drawing.Color]::FromArgb(0,0,255),
    [System.Drawing.Color]::FromArgb(255,255,0),
    [System.Drawing.Color]::FromArgb(0,255,255),
    [System.Drawing.Color]::FromArgb(255,0,255),
    [System.Drawing.Color]::FromArgb(255,255,255),
    [System.Drawing.Color]::FromArgb(0,0,0)
)
$barH = [int]($H * 0.30)
$barW = [int]($W / $bars.Count)
for ($i = 0; $i -lt $bars.Count; $i++) {
    $br = New-Object System.Drawing.SolidBrush($bars[$i])
    $g.FillRectangle($br, ($i * $barW), 0, $barW, $barH)
    $br.Dispose()
}

# --- Band 2: greyscale ramp in discrete steps (tests quantisation / bit depth) ---
$rampY = $barH
$rampH = [int]($H * 0.15)
$steps = 16
$stepW = [int]($W / $steps)
for ($i = 0; $i -lt $steps; $i++) {
    $v = [int]($i * 255 / ($steps - 1))
    $br = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb($v,$v,$v))
    $g.FillRectangle($br, ($i * $stepW), $rampY, $stepW, $rampH)
    $br.Dispose()
}

# --- Band 3: fine checkerboard, worst case for block compression ---
$ckY = $rampY + $rampH
$ckH = [int]($H * 0.25)
$cell = 8
$wBr = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::White)
for ($y = $ckY; $y -lt ($ckY + $ckH); $y += $cell) {
    for ($x = 0; $x -lt [int]($W/2); $x += $cell) {
        if ((([int](($x/$cell)) + [int]((($y-$ckY)/$cell))) % 2) -eq 0) {
            $g.FillRectangle($wBr, $x, $y, $cell, $cell)
        }
    }
}
# right half of band 3: solid mid grey, for a flat-vs-detail contrast on the same row
$midBr = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(128,128,128))
$g.FillRectangle($midBr, [int]($W/2), $ckY, [int]($W/2), $ckH)
$midBr.Dispose()

# --- Band 4: diagonals + corner markers, for geometry / stride verification ---
$dY = $ckY + $ckH
$dH = $H - $dY
$pen = New-Object System.Drawing.Pen([System.Drawing.Color]::White, 3)
for ($x = 0; $x -lt $W; $x += 120) {
    $g.DrawLine($pen, $x, $dY, ($x + $dH), $H)
}
$pen.Dispose()

# Corner markers: 64x64 pure red TL, green TR, blue BL, white BR - unambiguous origin/stride check
$mk = 64
$g.FillRectangle((New-Object System.Drawing.SolidBrush([System.Drawing.Color]::Red)),   0,        0,        $mk, $mk)
$g.FillRectangle((New-Object System.Drawing.SolidBrush([System.Drawing.Color]::Lime)),  ($W-$mk), 0,        $mk, $mk)
$g.FillRectangle((New-Object System.Drawing.SolidBrush([System.Drawing.Color]::Blue)),  0,        ($H-$mk), $mk, $mk)
$g.FillRectangle((New-Object System.Drawing.SolidBrush([System.Drawing.Color]::White)), ($W-$mk), ($H-$mk), $mk, $mk)

# Label so the reference image is self-describing
$font = New-Object System.Drawing.Font('Consolas', 28, [System.Drawing.FontStyle]::Bold)
$txt = "$Device  ${W}x${H}  $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')"
$g.DrawString($txt, $font, [System.Drawing.Brushes]::Black, 80, ($dY + 20))
$font.Dispose()
$g.Flush()

if ($SavePng) {
    $bmp.Save($SavePng, [System.Drawing.Imaging.ImageFormat]::Png)
    Write-Output "saved reference: $SavePng  (${W}x${H})"
}

$form = New-Object System.Windows.Forms.Form
$form.FormBorderStyle = [System.Windows.Forms.FormBorderStyle]::None
$form.StartPosition   = [System.Windows.Forms.FormStartPosition]::Manual
$form.Bounds          = $b
$form.TopMost         = $true
$form.BackgroundImage = $bmp
$form.BackgroundImageLayout = [System.Windows.Forms.ImageLayout]::None
$form.ShowInTaskbar    = $false
$form.Cursor           = [System.Windows.Forms.Cursors]::Default

$timer = New-Object System.Windows.Forms.Timer
$timer.Interval = $Seconds * 1000
$timer.Add_Tick({ $form.Close() })
$timer.Start()

Write-Output "showing pattern on $Device for $Seconds s"
[System.Windows.Forms.Application]::Run($form)
$g.Dispose(); $bmp.Dispose()
