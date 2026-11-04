#Requires -Version 5.1
<#
.SYNOPSIS
Seed a clean user home for the installer end-to-end job (reliable-installers
task 8.1, Requirement 10.2) and record a SHA-256 manifest of what was seeded.
.DESCRIPTION
Copies ci/e2e/seed/home byte for byte into -Destination (which must not exist
or be empty) and writes -Manifest in `sha256sum` format ("<hex>  <path>",
sorted, LF), so `sha256sum -c` run from the home also checks it.

The seed files are real-looking agent configs with unrelated settings,
comments and odd formatting: CRLF, tabs, a UTF-8 BOM, missing final newlines,
JSONC comments and trailing commas, other MCP servers and user hooks. Before
copying, the source bytes are checked against the committed reference
ci/e2e/seed.sha256, so a checkout that normalised line endings (core.autocrlf)
fails here instead of silently testing different bytes.

  e2e-seed-home.ps1 -Destination <home> -Manifest <file>
  e2e-seed-home.ps1 -UpdateReference     # after editing the seed files on purpose

Windows PowerShell 5.1 and PowerShell 7 compatible. Never touches anything
outside -Destination and -Manifest.
#>
[CmdletBinding()]
param([string]$Destination,[string]$Manifest,[string]$Source,[string]$Reference,[switch]$UpdateReference)
$ErrorActionPreference='Stop'

$script:RepoRoot=Split-Path -Parent $PSScriptRoot
if (-not $Source) {$Source=Join-Path $script:RepoRoot 'ci/e2e/seed/home'}
if (-not $Reference) {$Reference=Join-Path $script:RepoRoot 'ci/e2e/seed.sha256'}

# Relative, '/'-separated paths of every file under $Root, ordinal order.
function Get-SeedFiles([string]$Root) {
    $full=(Resolve-Path -LiteralPath $Root).ProviderPath.TrimEnd('\','/')
    $files=@(Get-ChildItem -LiteralPath $full -Recurse -Force -File | ForEach-Object {
        $_.FullName.Substring($full.Length+1).Replace('\','/')
    })
    [string[]]$sorted=$files
    [Array]::Sort($sorted,[StringComparer]::Ordinal)
    return $sorted
}
# Manifest text: "<sha256>  <relative path>\n" per file.
function Get-SeedManifestText([string]$Root) {
    $lines=foreach ($rel in (Get-SeedFiles $Root)) {
        $hash=(Get-FileHash -LiteralPath (Join-Path $Root $rel) -Algorithm SHA256).Hash.ToLowerInvariant()
        "$hash  $rel"
    }
    if (-not $lines) {throw "No seed files under $Root"}
    return (($lines -join "`n")+"`n")
}
function Write-Utf8NoBom([string]$Path,[string]$Text) {
    [IO.File]::WriteAllText($Path,$Text,(New-Object Text.UTF8Encoding($false)))
}
# Parse a manifest into an ordered map path -> sha256.
function Read-SeedManifest([string]$Path) {
    $map=[ordered]@{}
    foreach ($line in ([IO.File]::ReadAllText($Path) -split "`n")) {
        if (-not $line) {continue}
        if ($line -cnotmatch '^([0-9a-f]{64})  (.+)$') {throw "Bad manifest line in ${Path}: $line"}
        $map[$Matches[2]]=$Matches[1]
    }
    return $map
}

if ($MyInvocation.InvocationName -ne '.') {
    if ($UpdateReference) {
        Write-Utf8NoBom $Reference (Get-SeedManifestText $Source)
        Write-Host "Wrote $Reference"
        return
    }
    if (-not $Destination -or -not $Manifest) {throw 'Destination and Manifest are required'}
    $expected=[IO.File]::ReadAllText($Reference)
    $actual=Get-SeedManifestText $Source
    if ($actual -cne $expected) {
        throw "The seed files differ from $Reference (line endings normalised by the checkout, or an edit without -UpdateReference)"
    }
    if (Test-Path -LiteralPath $Destination) {
        if (@(Get-ChildItem -LiteralPath $Destination -Force).Count) {throw "The home $Destination is not empty"}
    } else {
        New-Item -ItemType Directory -Path $Destination | Out-Null
    }
    foreach ($rel in (Get-SeedFiles $Source)) {
        $target=Join-Path $Destination $rel
        $parent=Split-Path -Parent $target
        if (-not (Test-Path -LiteralPath $parent)) {New-Item -ItemType Directory -Path $parent -Force | Out-Null}
        [IO.File]::Copy((Join-Path $Source $rel),$target)
    }
    $seeded=Get-SeedManifestText $Destination
    if ($seeded -cne $expected) {throw 'The seeded home does not match the seed files byte for byte'}
    Write-Utf8NoBom $Manifest $seeded
    Write-Host "Seeded $(@(Get-SeedFiles $Destination).Count) files into $Destination; manifest $Manifest"
}
