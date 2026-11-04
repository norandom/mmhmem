#Requires -Version 5.1
<#
.SYNOPSIS
Build the per-target release archives and the SHA256SUMS manifest.
.DESCRIPTION
Archives are written by this script itself (a minimal ustar writer plus
.NET zip), so the result does not depend on which runner's tar or zip tool
is installed, and the executable bit of the Unix binary is set explicitly
(mode 0755) instead of depending on artifact transport, which drops modes.
Contents are deterministic: exactly one entry with a fixed name, fixed
mtime, root ownership. Windows PowerShell 5.1 and PowerShell 7 compatible.

Archive signing (task 5.2) rewrites archive bytes, so checksums are a
separate step: sign first, then call Write-ReleaseChecksums again.

Direct use:
  package-release-archives.ps1 -ReleaseTag v1.2.3 -OutputDirectory out `
    -WindowsBinary a -LinuxBinary b -MacIntelBinary c -MacArmBinary d
  package-release-archives.ps1 -ChecksumsOnly -OutputDirectory out
#>
[CmdletBinding()]
param([string]$ReleaseTag,[string]$OutputDirectory,[string]$WindowsBinary,[string]$LinuxBinary,[string]$MacIntelBinary,[string]$MacArmBinary,[switch]$ChecksumsOnly)

Add-Type -AssemblyName System.IO.Compression

function Assert-ArchiveTag([string]$Value) {
    if ($Value -cnotmatch '^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$') {throw 'Archive names require a stable vMAJOR.MINOR.PATCH tag'}
}
# Archive names are bound to the tag; the entry inside is fixed per target.
function Get-ReleaseArchiveTargets([string]$ReleaseTag) {
    Assert-ArchiveTag $ReleaseTag
    return @(
        [pscustomobject]@{Key='windows';Target='x86_64-pc-windows-msvc';Archive="mmhmem-$ReleaseTag-x86_64-pc-windows-msvc.zip";Entry='mmhmem.exe';Format='zip';Bare='mmhmem.exe'},
        [pscustomobject]@{Key='linux';Target='x86_64-unknown-linux-musl';Archive="mmhmem-$ReleaseTag-x86_64-unknown-linux-musl.tar.gz";Entry='mmhmem';Format='tar.gz';Bare='mmhmem-linux-x86_64'},
        [pscustomobject]@{Key='macos-x86_64';Target='x86_64-apple-darwin';Archive="mmhmem-$ReleaseTag-x86_64-apple-darwin.tar.gz";Entry='mmhmem';Format='tar.gz';Bare='mmhmem-macos-x86_64'},
        [pscustomobject]@{Key='macos-aarch64';Target='aarch64-apple-darwin';Archive="mmhmem-$ReleaseTag-aarch64-apple-darwin.tar.gz";Entry='mmhmem';Format='tar.gz';Bare='mmhmem-macos-aarch64'}
    )
}
function ConvertTo-TarOctal([long]$Value,[int]$Width) {
    # Zero-padded octal filling Width-1 digits plus a NUL terminator.
    $text=[Convert]::ToString($Value,8).PadLeft($Width-1,'0')
    if ($text.Length -ne $Width-1) {throw 'Value too large for tar field'}
    return [Text.Encoding]::ASCII.GetBytes($text+"`0")
}
function New-TarEntryBytes([string]$EntryName,[byte[]]$Content,[int]$Mode) {
    if ($EntryName.Length -eq 0 -or $EntryName.Length -gt 99 -or $EntryName -match '[/\\]') {throw 'Invalid tar entry name'}
    $header=New-Object byte[] 512
    function Set-Field([int]$Offset,[byte[]]$Bytes) {[Array]::Copy($Bytes,0,$header,$Offset,$Bytes.Length)}
    Set-Field 0 ([Text.Encoding]::ASCII.GetBytes($EntryName))
    Set-Field 100 (ConvertTo-TarOctal $Mode 8)
    Set-Field 108 (ConvertTo-TarOctal 0 8)
    Set-Field 116 (ConvertTo-TarOctal 0 8)
    Set-Field 124 (ConvertTo-TarOctal $Content.Length 12)
    Set-Field 136 (ConvertTo-TarOctal 0 12)
    for ($i=148;$i -lt 156;$i++) {$header[$i]=0x20}
    $header[156]=[byte][char]'0'
    Set-Field 257 ([Text.Encoding]::ASCII.GetBytes("ustar`0"+'00'))
    Set-Field 329 (ConvertTo-TarOctal 0 8)
    Set-Field 337 (ConvertTo-TarOctal 0 8)
    $sum=0;foreach ($b in $header) {$sum+=$b}
    Set-Field 148 ([Text.Encoding]::ASCII.GetBytes([Convert]::ToString($sum,8).PadLeft(6,'0')+"`0 "))
    $padding=(512-($Content.Length % 512)) % 512
    $stream=New-Object IO.MemoryStream
    $stream.Write($header,0,512)
    $stream.Write($Content,0,$Content.Length)
    $trailer=New-Object byte[] ($padding+1024)   # data padding plus end-of-archive blocks
    $stream.Write($trailer,0,$trailer.Length)
    return ,$stream.ToArray()
}
function New-ReleaseTarGz([string]$SourcePath,[string]$EntryName,[string]$Destination) {
    $content=[IO.File]::ReadAllBytes($SourcePath)
    $tar=New-TarEntryBytes $EntryName $content 0x1ED   # 0755
    $file=New-Object IO.FileStream($Destination,[IO.FileMode]::Create,[IO.FileAccess]::Write)
    try {
        $gzip=New-Object IO.Compression.GZipStream($file,[IO.Compression.CompressionLevel]::Optimal)
        try {$gzip.Write($tar,0,$tar.Length)} finally {$gzip.Dispose()}
    } finally {$file.Dispose()}
}
function New-ReleaseZip([string]$SourcePath,[string]$EntryName,[string]$Destination) {
    $content=[IO.File]::ReadAllBytes($SourcePath)
    $file=New-Object IO.FileStream($Destination,[IO.FileMode]::Create,[IO.FileAccess]::Write)
    try {
        $zip=New-Object IO.Compression.ZipArchive($file,[IO.Compression.ZipArchiveMode]::Create)
        try {
            $entry=$zip.CreateEntry($EntryName,[IO.Compression.CompressionLevel]::Optimal)
            $entry.LastWriteTime=[DateTimeOffset]::new(1980,1,1,0,0,0,[TimeSpan]::Zero)
            $out=$entry.Open()
            try {$out.Write($content,0,$content.Length)} finally {$out.Dispose()}
        } finally {$zip.Dispose()}
    } finally {$file.Dispose()}
}
# Sources: hashtable keyed windows / linux / macos-x86_64 / macos-aarch64 -> binary path.
function New-ReleaseArchives([string]$ReleaseTag,[hashtable]$Sources,[string]$OutputDirectory) {
    $targets=Get-ReleaseArchiveTargets $ReleaseTag
    foreach ($target in $targets) {
        $source=$Sources[$target.Key]
        if (-not $source -or -not (Test-Path -LiteralPath $source -PathType Leaf) -or (Get-Item -LiteralPath $source).Length -eq 0) {throw "Missing or empty binary for $($target.Target)"}
    }
    New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
    $written=New-Object System.Collections.Generic.List[string]
    foreach ($target in $targets) {
        $destination=Join-Path $OutputDirectory $target.Archive
        if ($target.Format -ceq 'zip') {New-ReleaseZip $Sources[$target.Key] $target.Entry $destination}
        else {New-ReleaseTarGz $Sources[$target.Key] $target.Entry $destination}
        $written.Add($destination)
    }
    return ,$written.ToArray()
}
# Re-runnable: rewrites SHA256SUMS from the files currently in the directory
# (or the explicit -Names), so it can run again after archives are signed.
function Write-ReleaseChecksums([string]$Directory,[string[]]$Names) {
    if (-not $Names) {
        $Names=@(Get-ChildItem -LiteralPath $Directory -File | Where-Object {$_.Name -cne 'SHA256SUMS' -and $_.Name -cne 'SHA256SUMS.sig'} | ForEach-Object {$_.Name})
    }
    $Names=[string[]]@($Names | Select-Object -Unique)
    [Array]::Sort($Names,[StringComparer]::Ordinal)
    if (-not $Names.Count) {throw 'No assets to hash'}
    $lines=foreach ($name in $Names) {
        if ($name -cmatch '[\\/\s]' -or $name -ceq 'SHA256SUMS' -or $name -ceq 'SHA256SUMS.sig') {throw "Invalid asset name: $name"}
        $path=Join-Path $Directory $name
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {throw "Missing asset: $name"}
        (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()+'  '+$name
    }
    $sumPath=Join-Path $Directory 'SHA256SUMS'
    [IO.File]::WriteAllText($sumPath,(($lines -join "`n")+"`n"),(New-Object Text.UTF8Encoding($false)))
    return $sumPath
}
if ($MyInvocation.InvocationName -ne '.') {
    $ErrorActionPreference='Stop'
    if (-not $OutputDirectory) {throw 'OutputDirectory is required'}
    if (-not $ChecksumsOnly) {
        $null=New-ReleaseArchives $ReleaseTag @{'windows'=$WindowsBinary;'linux'=$LinuxBinary;'macos-x86_64'=$MacIntelBinary;'macos-aarch64'=$MacArmBinary} $OutputDirectory
    }
    Write-Host "Wrote $(Write-ReleaseChecksums $OutputDirectory)"
}
