#Requires -Version 5.1
<#
Release signing (reliable-installers task 5.2). Dot-source for the functions.

Trust model (design section 2):
  1. every archive is signed in place with the pinned zipsign CLI; the signature
     context is the archive's FINAL published file name, so renaming breaks it;
  2. SHA256SUMS is computed AFTER signing, over all assets;
  3. SHA256SUMS.sig is a detached zipsign signature (magic header, count, then
     64-byte Ed25519ph signatures) over the exact SHA256SUMS bytes with the
     context "SHA256SUMS". zipsign-api verifies it with verify::read_signatures,
     Prehash::calculate and verify::find_match (the format task 5.4 reads).
Ed25519 is deterministic, so signing the same bytes twice yields identical bytes
and a retry passes Send-CiReleaseAsset's remote byte comparison.
The private key is only ever a base64 secret from the GitHub `release`
environment, written to a short-lived file and never printed.
#>
$script:ZipsignVersion='0.2.2'
$script:SigningSecretVariable='MMHMEM_RELEASE_SIGNING_KEY'
$script:ReleaseKeysSource=Join-Path (Join-Path $PSScriptRoot '..') 'src/setup/release_keys.rs'

function Get-ZipsignCommand {
    if ($env:ZIPSIGN) {return $env:ZIPSIGN}
    return 'zipsign'
}
function Invoke-Zipsign([string[]]$Arguments) {
    $previous=$ErrorActionPreference
    try {
        $ErrorActionPreference='Continue';$global:LASTEXITCODE=1
        $output=& (Get-ZipsignCommand) @Arguments 2>&1
        $code=$LASTEXITCODE
    } finally {$ErrorActionPreference=$previous}
    if ($code -ne 0) {throw "zipsign $($Arguments[0]) $($Arguments[1]) failed ($code): $(($output|ForEach-Object {$_.ToString()}) -join ' ')"}
    return (($output|ForEach-Object {$_.ToString()}) -join "`n")
}
function Assert-ZipsignVersion {
    $version=(Invoke-Zipsign @('--version')).Trim()
    if ($version -cne "zipsign $($script:ZipsignVersion)") {throw "Pinned zipsign $($script:ZipsignVersion) required, found '$version'"}
}
# Public keys embedded in the binary, parsed from release_keys.rs (0xNN byte
# arrays inside RELEASE_PUBLIC_KEYS). Returns lowercase hex strings.
function Get-EmbeddedReleaseKeys([string]$Source=$script:ReleaseKeysSource) {
    $text=[IO.File]::ReadAllText($Source)
    $block=[regex]::Match($text,'(?s)pub const RELEASE_PUBLIC_KEYS:\s*&\[\[u8;\s*32\]\]\s*=\s*&\[(.*?)\];')
    if (-not $block.Success) {throw 'RELEASE_PUBLIC_KEYS not found in release_keys.rs'}
    $body=[regex]::Replace($block.Groups[1].Value,'(?m)//.*$','')
    $keys=@()
    foreach ($m in [regex]::Matches($body,'\[([^\]]*)\]')) {
        $bytes=@([regex]::Matches($m.Groups[1].Value,'0x([0-9a-fA-F]{2})') | ForEach-Object {$_.Groups[1].Value.ToLowerInvariant()})
        if ($bytes.Count -ne 32) {throw 'Embedded release key must be 32 bytes written as 0xNN values'}
        $keys+=($bytes -join '')
    }
    return $keys
}
# Fails closed unless the secret decodes to a 64-byte zipsign key pair, the pinned
# zipsign is installed and the matching public key is embedded in the binary.
# Returns an object whose KeyFile/PublicKeyFile live in $Directory.
function Open-ReleaseSigning([string]$Directory,[string]$Secret=[Environment]::GetEnvironmentVariable($script:SigningSecretVariable),[string]$EmbeddedSource=$script:ReleaseKeysSource) {
    if (-not $Secret -or -not $Secret.Trim()) {throw "$($script:SigningSecretVariable) is not set; refusing to publish unsigned releases (configure the GitHub 'release' environment secret)"}
    try {$bytes=[Convert]::FromBase64String($Secret.Trim())} catch {throw "$($script:SigningSecretVariable) is not valid base64"}
    if ($bytes.Length -ne 64) {throw "$($script:SigningSecretVariable) must hold a 64-byte zipsign private key file"}
    Assert-ZipsignVersion
    New-Item -ItemType Directory -Path $Directory -Force | Out-Null
    $keyFile=Join-Path $Directory ('signing-'+[guid]::NewGuid().ToString('N')+'.key')
    $publicFile=Join-Path $Directory ('signing-'+[guid]::NewGuid().ToString('N')+'.pub')
    [IO.File]::WriteAllBytes($keyFile,$bytes)
    [Array]::Clear($bytes,0,$bytes.Length)
    $state=[pscustomobject]@{KeyFile=$keyFile;PublicKeyFile=$publicFile;PublicKeyHex=''}
    try {
        $null=Invoke-Zipsign @('gen-key','--extract',$keyFile,$publicFile,'--force')
        $state.PublicKeyHex=(([IO.File]::ReadAllBytes($publicFile) | ForEach-Object {$_.ToString('x2')}) -join '')
        if ($state.PublicKeyHex.Length -ne 64) {throw 'Unexpected public key length'}
        if ($state.PublicKeyHex -cnotin @(Get-EmbeddedReleaseKeys $EmbeddedSource)) {throw 'The signing key is not in RELEASE_PUBLIC_KEYS (src/setup/release_keys.rs); setup would reject this release'}
    } catch {
        Close-ReleaseSigning $state
        throw
    }
    return $state
}
function Close-ReleaseSigning($State) {
    if (-not $State) {return}
    foreach ($path in @($State.KeyFile,$State.PublicKeyFile)) {
        if ($path -and (Test-Path -LiteralPath $path)) {
            try {[IO.File]::WriteAllBytes($path,(New-Object byte[] 64))} catch {}
            Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue
        }
    }
}
function Get-ArchiveKind([string]$Path) {
    if ($Path -like '*.zip') {return 'zip'}
    if ($Path -like '*.tar.gz') {return 'tar'}
    throw "Not a release archive: $Path"
}
# Sign one archive in place. Context = the archive's final file name.
function Invoke-ReleaseArchiveSigning([string]$Archive,[string]$KeyFile) {
    $name=Split-Path -Leaf $Archive
    $temp=$Archive+'.signing'
    if (Test-Path -LiteralPath $temp) {Remove-Item -LiteralPath $temp -Force}
    try {
        $null=Invoke-Zipsign @('sign',(Get-ArchiveKind $Archive),'--context',$name,'--force','--output',$temp,$Archive,$KeyFile)
        Move-Item -LiteralPath $temp -Destination $Archive -Force
    } finally {if (Test-Path -LiteralPath $temp) {Remove-Item -LiteralPath $temp -Force}}
}
function Test-ReleaseArchiveSignature([string]$Archive,[string]$PublicKeyFile,[string]$ContextName) {
    if (-not $ContextName) {$ContextName=Split-Path -Leaf $Archive}
    try {$null=Invoke-Zipsign @('verify',(Get-ArchiveKind $Archive),'--quiet','--context',$ContextName,$Archive,$PublicKeyFile);return $true} catch {return $false}
}
# Sign every archive and prove each verifies under the public key before returning.
function Invoke-ReleaseSigning([string[]]$Archives,$Signing) {
    foreach ($archive in $Archives) {
        Invoke-ReleaseArchiveSigning $archive $Signing.KeyFile
        if (-not (Test-ReleaseArchiveSignature $archive $Signing.PublicKeyFile)) {throw "Signature self-check failed: $(Split-Path -Leaf $archive)"}
    }
}
# Detached signature over the exact SHA256SUMS bytes (context "SHA256SUMS").
function Write-ReleaseChecksumSignature([string]$Directory,$Signing) {
    $sums=Join-Path $Directory 'SHA256SUMS'
    $sig=Join-Path $Directory 'SHA256SUMS.sig'
    if (-not (Test-Path -LiteralPath $sums -PathType Leaf)) {throw 'SHA256SUMS must exist before it is signed'}
    $null=Invoke-Zipsign @('sign','separate','--context','SHA256SUMS','--force','--output',$sig,$sums,$Signing.KeyFile)
    if (-not (Test-ReleaseChecksumSignature $sums $sig $Signing.PublicKeyFile)) {throw 'SHA256SUMS.sig self-check failed'}
    return $sig
}
function Test-ReleaseChecksumSignature([string]$Sums,[string]$Signature,[string]$PublicKeyFile) {
    try {$null=Invoke-Zipsign @('verify','separate','--quiet','--context','SHA256SUMS',$Sums,$Signature,$PublicKeyFile);return $true} catch {return $false}
}
