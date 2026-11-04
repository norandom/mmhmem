#Requires -Version 5.1
# Offline checks for per-target archives and SHA256SUMS. No network, no GitHub.
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'package-release-archives.ps1')
Add-Type -AssemblyName System.IO.Compression.FileSystem
$root=Join-Path ([IO.Path]::GetTempPath()) ('pkg-archives-'+[guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $root | Out-Null
function Assert-Rejected([scriptblock]$Action) {
    $rejected=$false
    try {& $Action} catch {$rejected=$true}
    if (-not $rejected) {throw 'Invalid input was accepted'}
}
# Run tar from the archive's directory with a bare file name: GNU tar treats
# 'C:\...' as a remote host, bsdtar does not care.
$script:Tar='tar'
if ($env:SystemRoot -and (Test-Path -LiteralPath (Join-Path $env:SystemRoot 'System32\tar.exe'))) {$script:Tar=Join-Path $env:SystemRoot 'System32\tar.exe'}
function Invoke-Tar([string]$Directory,[string[]]$Arguments) {
    Push-Location -LiteralPath $Directory
    try {$output=@(& $script:Tar @Arguments);if ($LASTEXITCODE -ne 0) {throw "tar failed: $Arguments"};return ,$output} finally {Pop-Location}
}
function Assert-Equal($Actual,$Expected,[string]$What) {if ($Actual -cne $Expected) {throw "$What`: expected '$Expected' got '$Actual'"}}
try {
    $tag='v1.2.3'
    $src=Join-Path $root 'src';New-Item -ItemType Directory -Path $src | Out-Null
    $sources=@{}
    foreach ($key in @('windows','linux','macos-x86_64','macos-aarch64')) {
        $p=Join-Path $src $key
        # Larger than a tar block and not block-aligned, with non-text bytes.
        $bytes=New-Object byte[] (1500+$key.Length);for ($i=0;$i -lt $bytes.Length;$i++) {$bytes[$i]=($i*7+$key.Length) % 256}
        [IO.File]::WriteAllBytes($p,$bytes);$sources[$key]=$p
    }
    # Exact asset set and tag binding.
    $targets=Get-ReleaseArchiveTargets $tag
    Assert-Equal (($targets|ForEach-Object Archive|Sort-Object) -join ',') ('mmhmem-v1.2.3-aarch64-apple-darwin.tar.gz,mmhmem-v1.2.3-x86_64-apple-darwin.tar.gz,mmhmem-v1.2.3-x86_64-pc-windows-msvc.zip,mmhmem-v1.2.3-x86_64-unknown-linux-musl.tar.gz') 'archive names'
    foreach ($bad in @('main','v1.2','v01.2.3','v1.2.3-rc1','1.2.3')) {Assert-Rejected {Get-ReleaseArchiveTargets $bad}}
    $other=Get-ReleaseArchiveTargets 'v9.8.7'
    if (@($other|Where-Object {$_.Archive -notlike 'mmhmem-v9.8.7-*'}).Count) {throw 'Names not bound to the tag'}

    $out=Join-Path $root 'out'
    $written=New-ReleaseArchives $tag $sources $out
    Assert-Equal @($written).Count 4 'archive count'
    Assert-Equal ((Get-ChildItem $out -File|ForEach-Object Name|Sort-Object) -join ',') (($targets|ForEach-Object Archive|Sort-Object) -join ',') 'files on disk'
    Assert-Rejected {New-ReleaseArchives $tag @{'windows'=$sources['windows']} (Join-Path $root 'bad')}

    foreach ($t in $targets) {
        $archive=Join-Path $out $t.Archive
        $expected=[IO.File]::ReadAllBytes($sources[$t.Key])
        if ($t.Format -ceq 'zip') {
            $zip=[IO.Compression.ZipFile]::OpenRead($archive)
            try {
                Assert-Equal $zip.Entries.Count 1 'zip entry count'
                Assert-Equal $zip.Entries[0].FullName $t.Entry 'zip entry name'
                $ms=New-Object IO.MemoryStream;$s=$zip.Entries[0].Open();$s.CopyTo($ms);$s.Dispose()
                Assert-Equal ([Convert]::ToBase64String($ms.ToArray())) ([Convert]::ToBase64String($expected)) 'zip content'
            } finally {$zip.Dispose()}
        } else {
            $listing=Invoke-Tar $out @('-tvf',$t.Archive)
            Assert-Equal $listing.Count 1 'tar entry count'
            if ($listing[0] -notmatch '^-rwxr-xr-x\s' -or $listing[0] -notmatch '\smmhmem$') {throw "Unexpected tar entry: $($listing[0])"}
            if ((Invoke-Tar $out @('-tf',$t.Archive)).Count -ne 1) {throw 'tar has more than one entry'}
            $dest=Join-Path $root ('x-'+$t.Key);New-Item -ItemType Directory -Path $dest | Out-Null
            Copy-Item $archive $dest
            $null=Invoke-Tar $dest @('-xf',$t.Archive)
            Assert-Equal ([Convert]::ToBase64String([IO.File]::ReadAllBytes((Join-Path $dest 'mmhmem')))) ([Convert]::ToBase64String($expected)) 'tar content'
        }
    }
    # Deterministic: same inputs, same bytes.
    $out2=Join-Path $root 'out2'
    $null=New-ReleaseArchives $tag $sources $out2
    foreach ($t in $targets) {Assert-Equal (Get-FileHash (Join-Path $out2 $t.Archive)).Hash (Get-FileHash (Join-Path $out $t.Archive)).Hash "determinism $($t.Archive)"}

    # SHA256SUMS over every asset, including transition bare binaries.
    foreach ($bare in @('mmhmem.exe','mmhmem-linux-x86_64','mmhmem-macos-x86_64','mmhmem-macos-aarch64','verification.json')) {[IO.File]::WriteAllText((Join-Path $out $bare),"bare $bare")}
    $sums=Write-ReleaseChecksums $out
    $raw=[IO.File]::ReadAllText($sums)
    if (-not $raw.EndsWith("`n") -or $raw.Contains("`r")) {throw 'SHA256SUMS must use LF line endings'}
    $lines=@($raw.TrimEnd("`n") -split "`n")
    $assets=@(Get-ChildItem $out -File|Where-Object Name -cne 'SHA256SUMS')
    Assert-Equal $lines.Count 9 'SHA256SUMS line count'
    Assert-Equal $assets.Count 9 'asset count'
    foreach ($a in $assets) {
        $want=(Get-FileHash $a.FullName -Algorithm SHA256).Hash.ToLowerInvariant()+'  '+$a.Name
        if ($lines -cnotcontains $want) {throw "SHA256SUMS lacks or mis-hashes $($a.Name)"}
    }
    if ($lines -cnotmatch '^[0-9a-f]{64}  \S+$' | Measure-Object | ForEach-Object Count) {throw 'Malformed SHA256SUMS line'}
    # Re-running after an archive changes (as signing will) updates the manifest.
    $victim=Join-Path $out $targets[1].Archive
    $before=(Get-FileHash $victim).Hash.ToLowerInvariant()
    $bytes=[IO.File]::ReadAllBytes($victim)+[byte[]](1,2,3);[IO.File]::WriteAllBytes($victim,$bytes)
    $null=Write-ReleaseChecksums $out
    $after=(Get-FileHash $victim).Hash.ToLowerInvariant()
    $raw2=[IO.File]::ReadAllText($sums)
    if ($after -ceq $before -or $raw2.Contains($before) -or -not $raw2.Contains("$after  $($targets[1].Archive)")) {throw 'Re-hash did not update SHA256SUMS'}
    Assert-Equal (@($raw2.TrimEnd("`n") -split "`n").Count) 9 'line count after rehash'
    # SHA256SUMS never lists itself or its signature.
    [IO.File]::WriteAllText((Join-Path $out 'SHA256SUMS.sig'),'sig')
    $null=Write-ReleaseChecksums $out
    if ([IO.File]::ReadAllText($sums) -match 'SHA256SUMS') {throw 'SHA256SUMS lists itself or the signature'}
    Assert-Rejected {Write-ReleaseChecksums $out @('does-not-exist')}
    Assert-Rejected {Write-ReleaseChecksums $out @('SHA256SUMS')}
    Write-Host 'PASS: asset set, tag-bound names, single-entry archives, tar exec mode, determinism, SHA256SUMS and re-hash'
} finally {
    Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
}
