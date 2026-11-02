# Rebuild the compact UTF-8 / true-color ANSI artwork from the README image.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$root = Split-Path -Parent $PSScriptRoot
$source = [Drawing.Image]::FromFile((Join-Path $root 'assets/mmhmem.jpg'))
$width = 76
$height = 44
$bitmap = New-Object Drawing.Bitmap $width, $height
$graphics = [Drawing.Graphics]::FromImage($bitmap)
$glyphs = @{
    M = @('10001', '11011', '10101', '10001', '10001')
    H = @('10001', '10001', '11111', '10001', '10001')
    E = @('11111', '10000', '11110', '10000', '11111')
}
function Get-BannerPixel([int]$x, [int]$y) {
    if ($x -ge 28 -and $x -le 44 -and ($y -lt 6 -or $y -ge 38)) {
        $word = if ($y -lt 6) { 'MMH' } else { 'MEM' }
        $row = if ($y -lt 6) { $y } else { $y - 38 }
        $column = ($x - 28) % 6
        $letter = [string]$word[[int][math]::Floor(($x - 28) / 6)]
        if ($row -lt 5 -and $column -lt 5 -and $glyphs[$letter][$row][$column] -eq '1') {
            return [Drawing.Color]::White
        }
        return [Drawing.Color]::FromArgb(140, 0, 0)
    }
    # A small unsharp mask restores the cartoon's eye and outline contrast
    # after downsampling; it does not alter the source image.
    $pixel = $bitmap.GetPixel($x, $y)
    $channels = foreach ($channel in @('R', 'G', 'B')) {
        $sum = 0
        foreach ($dx in @(-1, 0, 1)) {
            foreach ($dy in @(-1, 0, 1)) {
                $neighbor = $bitmap.GetPixel([math]::Min($width - 1, [math]::Max(0, $x + $dx)), [math]::Min($height - 1, [math]::Max(0, $y + $dy)))
                $sum += $neighbor.$channel
            }
        }
        [int][math]::Min(255, [math]::Max(0, $pixel.$channel + 0.65 * ($pixel.$channel - $sum / 9)))
    }
    return [Drawing.Color]::FromArgb($channels[0], $channels[1], $channels[2])
}
try {
    $graphics.InterpolationMode = [Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $graphics.DrawImage($source, 0, 0, $width, $height)
    $art = New-Object Text.StringBuilder
    $escape = [char]27
    for ($y = 0; $y -lt $height; $y += 2) {
        [void]$art.Append('  ')
        for ($x = 0; $x -lt $width; $x++) {
            $top = Get-BannerPixel $x $y
            $bottom = Get-BannerPixel $x ($y + 1)
            [void]$art.Append("${escape}[38;2;$($top.R);$($top.G);$($top.B)m${escape}[48;2;$($bottom.R);$($bottom.G);$($bottom.B)m$([char]0x2580)")
        }
        [void]$art.Append("${escape}[0m`n")
    }
    [IO.File]::WriteAllText((Join-Path $root 'deploy/banner.ansi'), $art.ToString(), (New-Object Text.UTF8Encoding($false)))
} finally {
    $graphics.Dispose(); $bitmap.Dispose(); $source.Dispose()
}
