# Generate steady, controlled screen damage on one display.
# The DisplayLink protocol is damage-driven: a static screen sends almost nothing, so any
# experiment that needs a continuous stream of frame records has to keep something moving.
# A moving block on a flat background is predictable and easy to spot in a decoded frame.
param(
    [Parameter(Mandatory=$true)][string]$Device,
    [int]$Seconds = 60,
    [int]$Fps = 30
)

Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

$screen = [System.Windows.Forms.Screen]::AllScreens | Where-Object { $_.DeviceName -eq $Device }
if (-not $screen) { throw "no such display: $Device" }
$b = $screen.Bounds

$form = New-Object System.Windows.Forms.Form
$form.FormBorderStyle = [System.Windows.Forms.FormBorderStyle]::None
$form.StartPosition   = [System.Windows.Forms.FormStartPosition]::Manual
$form.Bounds          = $b
$form.TopMost         = $true
$form.ShowInTaskbar   = $false
$form.BackColor       = [System.Drawing.Color]::FromArgb(20,20,20)
$form.DoubleBuffered  = $true

$state = [pscustomobject]@{ X = 0; Y = [int]($b.Height/2) - 100; DX = 24; Hue = 0 }
$blockW = 260; $blockH = 200

$form.Add_Paint({
    param($sender, $e)
    $g = $e.Graphics
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::None

    # moving colour block
    $c = [System.Drawing.Color]::FromArgb(
            (128 + 127 * [Math]::Sin($state.Hue * 0.05)),
            (128 + 127 * [Math]::Sin($state.Hue * 0.05 + 2.1)),
            (128 + 127 * [Math]::Sin($state.Hue * 0.05 + 4.2)))
    $br = New-Object System.Drawing.SolidBrush($c)
    $g.FillRectangle($br, $state.X, $state.Y, $blockW, $blockH)
    $br.Dispose()

    # static reference bars down the left edge, so a decoded frame always has known colours
    $ref = @([System.Drawing.Color]::Red, [System.Drawing.Color]::Lime, [System.Drawing.Color]::Blue,
             [System.Drawing.Color]::White)
    for ($i = 0; $i -lt $ref.Count; $i++) {
        $rb = New-Object System.Drawing.SolidBrush($ref[$i])
        $g.FillRectangle($rb, 0, ($i * 120), 100, 110)
        $rb.Dispose()
    }
})

$timer = New-Object System.Windows.Forms.Timer
$timer.Interval = [int](1000 / $Fps)
$timer.Add_Tick({
    $state.X += $state.DX
    $state.Hue += 1
    if ($state.X -lt 0 -or ($state.X + $blockW) -gt $b.Width) { $state.DX = -$state.DX }
    $form.Invalidate()
})
$timer.Start()

$stopTimer = New-Object System.Windows.Forms.Timer
$stopTimer.Interval = $Seconds * 1000
$stopTimer.Add_Tick({ $form.Close() })
$stopTimer.Start()

Write-Output "animating on $Device for $Seconds s at $Fps fps"
[System.Windows.Forms.Application]::Run($form)
