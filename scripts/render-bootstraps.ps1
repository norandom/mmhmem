#Requires -Version 5.1
<#
.SYNOPSIS
Render the install.sh / install.ps1 bootstrap scripts for one release.
.DESCRIPTION
Fills deploy/bootstrap/install.{sh,ps1}.tmpl with the tag, the base URL and
the SHA-256 of each staged (already signed) archive. Placeholders look like
@@NAME@@ and are exactly:
  @@TAG@@  @@BASE_URL@@  @@SHA256_WINDOWS@@  @@SHA256_LINUX@@
  @@SHA256_MACOS_X86_64@@  @@SHA256_MACOS_AARCH64@@
An unknown placeholder, a missing archive or a leftover placeholder is an
error. The base URL defaults to the GitHub release download URL of the tag.
-BaseUrl exists only for tests and CI (the end-to-end job serves the same
rendered scripts from a local server); release publishing never passes it.

Run after archive signing (hashes are of the final bytes) and before
Write-ReleaseChecksums so SHA256SUMS covers the scripts. Output files are
LF-only, UTF-8 without BOM.

  render-bootstraps.ps1 -ReleaseTag v1.2.3 -ArchiveDirectory staging -OutputDirectory staging
#>
[CmdletBinding()]
param([string]$ReleaseTag,[string]$ArchiveDirectory,[string]$OutputDirectory,[string]$BaseUrl)

. (Join-Path $PSScriptRoot 'package-release-archives.ps1')

$script:BootstrapRepository='https://github.com/norandom/mmhmem/releases/download'
# Placeholder -> archive Key from Get-ReleaseArchiveTargets (null = not a hash).
$script:BootstrapPlaceholders=[ordered]@{
    'TAG'=$null;'BASE_URL'=$null
    'SHA256_WINDOWS'='windows';'SHA256_LINUX'='linux'
    'SHA256_MACOS_X86_64'='macos-x86_64';'SHA256_MACOS_AARCH64'='macos-aarch64'
}
$script:BootstrapTemplates=[ordered]@{'install.sh'='install.sh.tmpl';'install.ps1'='install.ps1.tmpl'}

function Get-BootstrapTemplateDirectory {Join-Path (Split-Path -Parent $PSScriptRoot) 'deploy/bootstrap'}

function Expand-BootstrapTemplate([string]$Name,[string]$Text,[hashtable]$Values) {
    foreach ($m in [regex]::Matches($Text,'@@([^@\s]*)@@')) {
        if (-not $Values.ContainsKey($m.Groups[1].Value)) {throw "$Name has unknown placeholder $($m.Value)"}
    }
    foreach ($key in $Values.Keys) {$Text=$Text.Replace("@@$key@@",[string]$Values[$key])}
    if ($Text -match '@@') {throw "$Name still contains a placeholder after rendering"}
    return $Text
}
# Returns the names of the written scripts (install.sh, install.ps1).
function Write-RenderedBootstraps([string]$ReleaseTag,[string]$ArchiveDirectory,[string]$OutputDirectory,[string]$BaseUrl) {
    $targets=Get-ReleaseArchiveTargets $ReleaseTag
    if (-not $BaseUrl) {$BaseUrl="$script:BootstrapRepository/$ReleaseTag"}
    # The value lands inside single-quoted script strings: allow only plain URL characters.
    if ($BaseUrl -cnotmatch '^https?://[A-Za-z0-9._~:/-]+[A-Za-z0-9_~-]$') {throw 'Invalid base URL'}
    $values=@{'TAG'=$ReleaseTag;'BASE_URL'=$BaseUrl}
    foreach ($name in $script:BootstrapPlaceholders.Keys) {
        $key=$script:BootstrapPlaceholders[$name]
        if (-not $key) {continue}
        $target=@($targets | Where-Object {$_.Key -ceq $key})[0]
        $path=Join-Path $ArchiveDirectory $target.Archive
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {throw "Missing archive for hashing: $($target.Archive)"}
        $hash=(Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($hash -cnotmatch '^[0-9a-f]{64}$') {throw "Bad hash for $($target.Archive)"}
        $values[$name]=$hash
    }
    $templateDirectory=Get-BootstrapTemplateDirectory
    $rendered=[ordered]@{}
    foreach ($output in $script:BootstrapTemplates.Keys) {
        $templatePath=Join-Path $templateDirectory $script:BootstrapTemplates[$output]
        if (-not (Test-Path -LiteralPath $templatePath -PathType Leaf)) {throw "Missing template: $templatePath"}
        $text=[IO.File]::ReadAllText($templatePath).Replace("`r`n","`n")
        $rendered[$output]=Expand-BootstrapTemplate $output $text $values
    }
    # Write only after every script rendered, so a failure leaves no partial output.
    New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
    foreach ($output in $rendered.Keys) {
        [IO.File]::WriteAllText((Join-Path $OutputDirectory $output),$rendered[$output],(New-Object Text.UTF8Encoding($false)))
    }
    return ,[string[]]@($rendered.Keys)
}
if ($MyInvocation.InvocationName -ne '.') {
    $ErrorActionPreference='Stop'
    if (-not $ReleaseTag -or -not $ArchiveDirectory -or -not $OutputDirectory) {throw 'ReleaseTag, ArchiveDirectory and OutputDirectory are required'}
    $names=Write-RenderedBootstraps $ReleaseTag $ArchiveDirectory $OutputDirectory $BaseUrl
    Write-Host "Rendered $($names -join ', ') into $OutputDirectory"
}
