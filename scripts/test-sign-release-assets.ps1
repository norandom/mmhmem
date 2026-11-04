#Requires -Version 5.1
# Offline checks for release signing (task 5.2). Uses throwaway keys generated
# here and the pinned zipsign CLI (ZIPSIGN env var or PATH). No GitHub access.
$ErrorActionPreference='Stop'
$tag='v1.2.3'
. (Join-Path $PSScriptRoot 'publish-ci-release.ps1') -Tag $tag -RunId '123'
Add-Type -AssemblyName System.IO.Compression.FileSystem
function Assert-Rejected([scriptblock]$Action,[string]$Pattern) {
    $message=$null
    try {& $Action} catch {$message=$_.Exception.Message}
    if ($null -eq $message) {throw 'Invalid input was accepted'}
    if ($Pattern -and $message -notmatch $Pattern) {throw "Rejected for the wrong reason: $message"}
}
function Assert-True($Condition,[string]$What) {if (-not $Condition) {throw "Assertion failed: $What"}}
function Get-Sha([string]$Path) {(Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()}
function ConvertTo-Hex([byte[]]$Bytes) {(($Bytes|ForEach-Object {$_.ToString('x2')}) -join '')}
function Get-RustKeyList([string[]]$Hex) {
    $entries=foreach ($h in $Hex) {'    ['+((0..31|ForEach-Object {'0x'+$h.Substring($_*2,2)}) -join ', ')+'],'}
    return "pub const RELEASE_PUBLIC_KEYS: &[[u8; 32]] = &[`n// owner keys`n"+($entries -join "`n")+"`n];`n"
}
try {Assert-ZipsignVersion} catch {throw "$_ -- install: cargo install zipsign --version =$($script:ZipsignVersion) --locked"}
$root=Join-Path ([IO.Path]::GetTempPath()) ('sign-test-'+[guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $root | Out-Null
$oldSecret=[Environment]::GetEnvironmentVariable($script:SigningSecretVariable)
try {
    # Throwaway key pairs A (release) and B (foreign).
    foreach ($k in 'a','b') {$null=Invoke-Zipsign @('gen-key',(Join-Path $root "$k.key"),(Join-Path $root "$k.pub"))}
    $pubA=Join-Path $root 'a.pub';$pubB=Join-Path $root 'b.pub'
    $hexA=ConvertTo-Hex ([IO.File]::ReadAllBytes($pubA))
    $hexB=ConvertTo-Hex ([IO.File]::ReadAllBytes($pubB))
    $secretA=[Convert]::ToBase64String([IO.File]::ReadAllBytes((Join-Path $root 'a.key')))
    $rsA=Join-Path $root 'keys-a.rs';[IO.File]::WriteAllText($rsA,(Get-RustKeyList @($hexA)))
    $rsEmpty=Join-Path $root 'keys-empty.rs';[IO.File]::WriteAllText($rsEmpty,"pub const RELEASE_PUBLIC_KEYS: &[[u8; 32]] = &[`n    // none yet`n];`n")
    $rsB=Join-Path $root 'keys-b.rs';[IO.File]::WriteAllText($rsB,(Get-RustKeyList @($hexB)))
    # The committed list must parse (empty until the owner adds the key).
    $null=Get-EmbeddedReleaseKeys

    # Archives as the publisher builds them.
    $src=Join-Path $root 'src';New-Item -ItemType Directory -Path $src | Out-Null
    $sources=@{}
    foreach ($key in @('windows','linux','macos-x86_64','macos-aarch64')) {
        $bytes=New-Object byte[] (3000+$key.Length);for ($i=0;$i -lt $bytes.Length;$i++) {$bytes[$i]=($i*7+$key.Length) % 256}
        $p=Join-Path $src $key;[IO.File]::WriteAllBytes($p,$bytes);$sources[$key]=$p
    }
    $out=Join-Path $root 'out'
    $archives=@(New-ReleaseArchives $tag $sources $out | ForEach-Object {$_})
    Assert-True ($archives.Count -eq 4) 'four archives'
    $unsignedHash=@{};foreach ($a in $archives) {$unsignedHash[$a]=Get-Sha $a}

    # Open-ReleaseSigning: key must be present, well formed and embedded.
    $keyDir=Join-Path $root 'keys'
    Assert-Rejected {Open-ReleaseSigning $keyDir '' $rsA} 'not set'
    Assert-Rejected {Open-ReleaseSigning $keyDir '   ' $rsA} 'not set'
    Assert-Rejected {Open-ReleaseSigning $keyDir '!!notbase64' $rsA} 'base64'
    Assert-Rejected {Open-ReleaseSigning $keyDir ([Convert]::ToBase64String((New-Object byte[] 10))) $rsA} '64-byte'
    Assert-Rejected {Open-ReleaseSigning $keyDir $secretA $rsEmpty} 'not in RELEASE_PUBLIC_KEYS'
    Assert-Rejected {Open-ReleaseSigning $keyDir $secretA $rsB} 'not in RELEASE_PUBLIC_KEYS'
    Assert-True (-not @(Get-ChildItem -LiteralPath $keyDir -File -ErrorAction SilentlyContinue).Count) 'failed open leaves no key file'
    $signing=Open-ReleaseSigning $keyDir $secretA $rsA
    Assert-True ($signing.PublicKeyHex -ceq $hexA) 'extracted public key matches'

    # Every archive is signed, verifies with A, fails with B, still extracts.
    Invoke-ReleaseSigning $archives $signing
    foreach ($a in $archives) {
        $leaf=Split-Path -Leaf $a
        Assert-True ((Get-Sha $a) -cne $unsignedHash[$a]) "signed bytes differ: $leaf"
        Assert-True (Test-ReleaseArchiveSignature $a $pubA) "verifies with release key: $leaf"
        Assert-True (-not (Test-ReleaseArchiveSignature $a $pubB)) "fails with other key: $leaf"
        # Context binding: a renamed copy no longer verifies under its new name.
        $renamed=Join-Path $root ('renamed-'+$leaf)
        Copy-Item -LiteralPath $a -Destination $renamed
        Assert-True (-not (Test-ReleaseArchiveSignature $renamed $pubA)) "rename breaks verification: $leaf"
        Assert-True (Test-ReleaseArchiveSignature $renamed $pubA $leaf) 'same bytes verify with the original context'
    }
    # Signed archives still extract.
    $zip=@($archives|Where-Object {$_ -like '*.zip'})[0]
    $zr=[IO.Compression.ZipFile]::OpenRead($zip)
    try {Assert-True ($zr.Entries.Count -eq 1 -and $zr.Entries[0].Length -eq (Get-Item $sources['windows']).Length) 'zip extracts after signing'} finally {$zr.Dispose()}
    $tar='tar';if ($env:SystemRoot -and (Test-Path (Join-Path $env:SystemRoot 'System32\tar.exe'))) {$tar=Join-Path $env:SystemRoot 'System32\tar.exe'}
    foreach ($tgz in @($archives|Where-Object {$_ -like '*.tar.gz'})) {
        Push-Location -LiteralPath (Split-Path -Parent $tgz)
        try {$listing=@(& $tar -tzf (Split-Path -Leaf $tgz));Assert-True ($LASTEXITCODE -eq 0 -and $listing.Count -eq 1) 'tar.gz lists after signing'} finally {Pop-Location}
    }
    # A modified archive (same name) fails verification.
    $tampered=Join-Path $root 'tamper';New-Item -ItemType Directory -Path $tampered|Out-Null
    $victim=Join-Path $tampered (Split-Path -Leaf $zip);$bytes=[IO.File]::ReadAllBytes($zip);$bytes[10]=$bytes[10] -bxor 1;[IO.File]::WriteAllBytes($victim,$bytes)
    Assert-True (-not (Test-ReleaseArchiveSignature $victim $pubA)) 'modified archive fails'

    # Determinism: signing the same unsigned bytes twice yields identical bytes,
    # equal to the first signing (an exact retry passes the remote byte check).
    $rebuilt=@(New-ReleaseArchives $tag $sources (Join-Path $root "rebuilt") | ForEach-Object {$_})
    $d1=Join-Path $root 'd1';$d2=Join-Path $root 'd2';New-Item -ItemType Directory -Path $d1,$d2|Out-Null
    foreach ($orig in $rebuilt) {
        $leaf=Split-Path -Leaf $orig
        $f1=Join-Path $d1 $leaf;$f2=Join-Path $d2 $leaf
        Copy-Item $orig $f1;Copy-Item $orig $f2
        Invoke-ReleaseArchiveSigning $f1 $signing.KeyFile;Invoke-ReleaseArchiveSigning $f2 $signing.KeyFile
        Assert-True ((Get-Sha $f1) -ceq (Get-Sha $f2)) "signing twice is identical: $leaf"
        Assert-True ((Get-Sha $f1) -ceq (Get-Sha (Join-Path $out $leaf))) "rebuilt+signed equals first signed: $leaf"
    }

    # SHA256SUMS is computed after signing and over all assets.
    $names=[string[]]@($archives|ForEach-Object {Split-Path -Leaf $_})
    $sumPath=Write-ReleaseChecksums $out $names
    $lines=@([IO.File]::ReadAllText($sumPath).TrimEnd("`n") -split "`n")
    foreach ($a in $archives) {
        $n=Split-Path -Leaf $a
        Assert-True ($lines -ccontains ((Get-Sha $a)+'  '+$n)) "SHA256SUMS has signed hash: $n"
        Assert-True ($lines -cnotcontains ($unsignedHash[$a]+'  '+$n)) "SHA256SUMS lacks unsigned hash: $n"
    }
    # SHA256SUMS.sig verifies over the exact bytes and fails when one byte changes.
    $sigPath=Write-ReleaseChecksumSignature $out $signing
    Assert-True ((Split-Path -Leaf $sigPath) -ceq 'SHA256SUMS.sig') 'signature file name'
    Assert-True (Test-ReleaseChecksumSignature $sumPath $sigPath $pubA) 'SHA256SUMS.sig verifies'
    Assert-True (-not (Test-ReleaseChecksumSignature $sumPath $sigPath $pubB)) 'SHA256SUMS.sig fails with other key'
    Assert-True ([IO.File]::ReadAllBytes($sigPath).Length -gt 64) 'zipsign detached format (header + signature)'
    $mod=Join-Path $root 'mod';New-Item -ItemType Directory -Path $mod|Out-Null
    $sumBytes=[IO.File]::ReadAllBytes($sumPath);$sumBytes[0]=$sumBytes[0] -bxor 1;$modSums=Join-Path $mod 'SHA256SUMS';[IO.File]::WriteAllBytes($modSums,$sumBytes)
    Assert-True (-not (Test-ReleaseChecksumSignature $modSums $sigPath $pubA)) 'one changed byte fails'
    $before=Get-Sha $sigPath;$null=Write-ReleaseChecksumSignature $out $signing
    Assert-True ((Get-Sha $sigPath) -ceq $before) 'SHA256SUMS.sig is deterministic'
    Close-ReleaseSigning $signing
    Assert-True (-not (Test-Path -LiteralPath $signing.KeyFile)) 'private key file removed'

    # Missing or malformed secret: the publisher fails closed before any gh call.
    $script:GhCalls=0
    function Invoke-ReleaseGh([string[]]$Arguments) {$script:GhCalls++;throw 'gh must not be called'}
    function Get-ReleaseApi([string]$Endpoint) {$script:GhCalls++;throw 'API must not be called'}
    [Environment]::SetEnvironmentVariable($script:SigningSecretVariable,$null)
    Assert-Rejected {Invoke-CiRelease $tag '123'} 'not set'
    [Environment]::SetEnvironmentVariable($script:SigningSecretVariable,[Convert]::ToBase64String((New-Object byte[] 10)))
    Assert-Rejected {Invoke-CiRelease $tag '123'} '64-byte'
    Assert-True ($script:GhCalls -eq 0) 'no GitHub call without a valid signing key'

    # Attestation subjects handed to the workflow: forward slashes, multi-line output.
    $oldOut=$env:GITHUB_OUTPUT;$outFile=Join-Path $root 'github-output';$env:GITHUB_OUTPUT=$outFile
    try {Write-AttestationSubjects @('C:\a\one.zip','C:\a\two.tar.gz')} finally {$env:GITHUB_OUTPUT=$oldOut}
    $o=[IO.File]::ReadAllText($outFile)
    Assert-True ($o -ceq "archives<<MMHMEM_ARCHIVES`nC:/a/one.zip`nC:/a/two.tar.gz`nMMHMEM_ARCHIVES`n") 'attestation subject output'

    # Order inside the publisher: key check first, sign, hash, sign sums, uploads, sums, sig last.
    $text=[IO.File]::ReadAllText((Join-Path $PSScriptRoot 'publish-ci-release.ps1'))
    $body=$text.Substring($text.IndexOf('function Invoke-CiRelease'))
    $order='Open-ReleaseSigning','Resolve-ReleaseTag $ReleaseTag','Invoke-LinuxReleaseDelivery','Initialize-CiReleaseAssets','Invoke-ReleaseSigning','Write-ReleaseChecksums','Write-ReleaseChecksumSignature','Send-CiReleaseAsset $ReleaseTag (Join-Path','Send-CiReleaseAsset $ReleaseTag $sumPath','Send-CiReleaseAsset $ReleaseTag $sigPath'
    $last=-1
    foreach ($marker in $order) {
        $at=$body.IndexOf($marker,[Math]::Max($last,0))
        if ($at -lt 0 -or $at -le $last) {throw "Publisher step out of order: $marker"}
        $last=$at
    }
    Write-Host 'PASS: signed archives verify/extract, rename+tamper rejected, sums after signing, SHA256SUMS.sig, determinism, fail-closed without secret'
} finally {
    [Environment]::SetEnvironmentVariable($script:SigningSecretVariable,$oldSecret)
    Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
}
