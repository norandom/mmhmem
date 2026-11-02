# Rebuild the compact UTF-8 / true-color ANSI artwork from the README image.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$root = Split-Path -Parent $PSScriptRoot
$source = [Drawing.Image]::FromFile((Join-Path $root 'assets/mmhmem.jpg'))
$bitmap = New-Object Drawing.Bitmap 56, 32
$graphics = [Drawing.Graphics]::FromImage($bitmap)
try {
    $graphics.InterpolationMode = [Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $graphics.DrawImage($source, 0, 0, 56, 32)
    $art = New-Object Text.StringBuilder
    $escape = [char]27
    for ($y = 0; $y -lt 32; $y += 2) {
        [void]$art.Append('  ')
        for ($x = 0; $x -lt 56; $x++) {
            $top = $bitmap.GetPixel($x, $y)
            $bottom = $bitmap.GetPixel($x, $y + 1)
            [void]$art.Append("${escape}[38;2;$($top.R);$($top.G);$($top.B)m${escape}[48;2;$($bottom.R);$($bottom.G);$($bottom.B)m$([char]0x2580)")
        }
        [void]$art.Append("${escape}[0m`n")
    }
    [IO.File]::WriteAllText((Join-Path $root 'deploy/banner.ansi'), $art.ToString(), (New-Object Text.UTF8Encoding($false)))
} finally {
    $graphics.Dispose(); $bitmap.Dispose(); $source.Dispose()
}
