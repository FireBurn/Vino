# Dump every attached monitor's raw EDID, and decode the bit of it that matters here.
#
# Why: Windows tone-maps HDR content to the sink's DECLARED peak luminance before the DisplayLink
# driver ever sees a pixel. So the sink's CTA-861 HDR Static Metadata block is what says how much
# of the 10-bit range a capture can possibly contain -- without it, a decoded wire value cannot be
# turned back into a luminance and the whole analysis is guesswork.
#
#   .\edid.ps1                        -> summarise every monitor
#   .\edid.ps1 -OutDir ..\out\edid    -> also write the raw blobs for the Linux side
#
# The blobs are what to keep: `edid-decode` on Linux reads far more of them than this does, and
# ⚠ its ST 2084 line does NOT contain the string "HDR", so grepping for that hides exactly the
# thing you came for.
param(
    [string]$OutDir = ''
)

$ErrorActionPreference = 'Stop'

if ($OutDir -and -not (Test-Path $OutDir)) { New-Item -ItemType Directory -Path $OutDir | Out-Null }

function Decode-Edid([byte[]]$e, [string]$label) {
    if ($e.Length -lt 128) { Write-Host "$label : EDID too short ($($e.Length) bytes)"; return }

    # Manufacturer id is three 5-bit letters packed big-endian at bytes 8..9.
    $m = ($e[8] -shl 8) -bor $e[9]
    $mfg = [string][char](64 + (($m -shr 10) -band 0x1f)) +
           [char](64 + (($m -shr 5) -band 0x1f)) + [char](64 + ($m -band 0x1f))
    $prod = '{0:X2}{1:X2}' -f $e[11], $e[10]

    # Monitor name lives in a 0xFC descriptor in the 18-byte descriptor block at 54..125.
    $name = ''
    for ($i = 54; $i -le 108; $i += 18) {
        if ($e[$i] -eq 0 -and $e[$i+1] -eq 0 -and $e[$i+3] -eq 0xFC) {
            $name = ([System.Text.Encoding]::ASCII.GetString($e[($i+5)..($i+17)])).Trim("`0", ' ', "`n")
        }
    }

    Write-Host ("{0} : {1} {2} {3}  ({4} bytes)" -f $label, $mfg, $prod, $name, $e.Length)

    # CTA-861 extension blocks carry the HDR data. Walk each 128-byte block whose tag is 0x02.
    for ($b = 128; $b + 128 -le $e.Length; $b += 128) {
        if ($e[$b] -ne 0x02) { continue }
        $dtd = $e[$b + 2]                      # offset of the first detailed timing = end of DBC
        if ($dtd -le 4) { continue }
        $p = $b + 4
        while ($p -lt $b + $dtd) {
            $len = $e[$p] -band 0x1f
            $tag = ($e[$p] -shr 5) -band 0x07
            if ($len -eq 0) { break }
            if ($tag -eq 7) {                  # extended tag
                $ext = $e[$p + 1]
                if ($ext -eq 6 -and $len -ge 3) {
                    # HDR Static Metadata Data Block.
                    $eotf = $e[$p + 2]
                    $eotfs = @()
                    if ($eotf -band 0x01) { $eotfs += 'SDR gamma' }
                    if ($eotf -band 0x02) { $eotfs += 'HDR gamma' }
                    if ($eotf -band 0x04) { $eotfs += 'SMPTE ST 2084 (PQ)' }
                    if ($eotf -band 0x08) { $eotfs += 'HLG' }
                    Write-Host ("    HDR static metadata: EOTF 0x{0:X2} = {1}" -f $eotf, ($eotfs -join ', '))
                    # Luminance bytes are optional and coded: cd/m2 = 50 * 2^(v/32),
                    # min = max * (v/255)^2 / 100.
                    if ($len -ge 5) {
                        $vmax = $e[$p + 4]
                        $peak = 50.0 * [Math]::Pow(2.0, $vmax / 32.0)
                        Write-Host ("    declared peak      : {0:N1} cd/m2 (byte {1})" -f $peak, $vmax)
                        if ($len -ge 6) {
                            $vavg = $e[$p + 5]
                            Write-Host ("    declared MaxFALL   : {0:N1} cd/m2" -f (50.0 * [Math]::Pow(2.0, $vavg / 32.0)))
                        }
                        if ($len -ge 7) {
                            $vmin = $e[$p + 6]
                            Write-Host ("    declared min       : {0:N4} cd/m2" -f ($peak * [Math]::Pow($vmin / 255.0, 2) / 100.0))
                        }
                    } else {
                        Write-Host '    ⚠ no luminance bytes -- the sink declares PQ but not how bright it is'
                    }
                } elseif ($ext -eq 5) {
                    Write-Host ("    colorimetry        : 0x{0:X2}{1}" -f $e[$p + 2],
                                $(if ($e[$p + 2] -band 0xC0) { ' (includes BT.2020)' } else { '' }))
                }
            }
            $p += $len + 1
        }
    }
}

$root = 'HKLM:\SYSTEM\CurrentControlSet\Enum\DISPLAY'
$n = 0
foreach ($mon in Get-ChildItem $root -ErrorAction SilentlyContinue) {
    foreach ($inst in Get-ChildItem $mon.PSPath -ErrorAction SilentlyContinue) {
        $dp = Join-Path $inst.PSPath 'Device Parameters'
        $edid = (Get-ItemProperty -Path $dp -Name EDID -ErrorAction SilentlyContinue).EDID
        if (-not $edid) { continue }
        # Only monitors Windows currently has attached carry a live driver key; the enum keeps
        # every monitor ever plugged in, and a stale one is worse than no answer.
        $active = (Get-ItemProperty -Path $inst.PSPath -Name Driver -ErrorAction SilentlyContinue).Driver
        $label = '{0}\{1}{2}' -f $mon.PSChildName, $inst.PSChildName, $(if ($active) { '' } else { '  [STALE]' })
        Decode-Edid $edid $label
        if ($OutDir) {
            $f = Join-Path $OutDir ("{0}-{1}.edid.bin" -f $mon.PSChildName, $inst.PSChildName)
            [IO.File]::WriteAllBytes($f, $edid)
            $n++
        }
    }
}
if ($OutDir) { Write-Host "`nwrote $n EDID blob(s) to $OutDir" }
Write-Host "`nNote: this walks the registry, which remembers monitors that are no longer attached."
Write-Host "Entries without a live driver key are marked [STALE]. Match the TV by its name above."
