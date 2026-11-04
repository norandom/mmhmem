#requires -Version 5.1
# mmhmem-transition-delegator
#
# TRANSITION INSTALLER. This file replaces the old repository-HEAD installer so
# that the one-liner
#   irm https://raw.githubusercontent.com/norandom/mmhmem/HEAD/install.ps1 | iex
# keeps working while HEAD moves ahead of the published release. It installs the
# latest published release exactly as that release's own installer does, with
# every download pinned to that release's tag:
#   1. resolve the latest stable release tag (vX.Y.Z) from the GitHub API;
#   2. fetch that tag's own install.ps1;
#   3. rewrite its /HEAD and releases/latest/download URLs to the tag;
#   4. run it with your parameters.
# It refuses, changing nothing, if the tag lookup or fetch fails, if a /HEAD
# reference would remain, or if the fetched script is itself this delegator.
# Task 9.2 in .kiro/specs/reliable-installers replaces this file with a stub that
# points at the new release assets.
[CmdletBinding()]
param(
    [string]$Agent,
    [string]$WorkerModel,
    [string]$WorkerUrl,
    [string]$WorkerApiKey,
    [switch]$Remove,
    [switch]$DryRun
)
$ErrorActionPreference = 'Stop'
try { [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12 } catch { }
function Stop-Delegator([string]$Message) { throw "mmhmem: $Message Nothing was changed." }
try {
    $release = Invoke-RestMethod -UseBasicParsing -Uri 'https://api.github.com/repos/norandom/mmhmem/releases/latest'
    $tag = [string]$release.tag_name
} catch { Stop-Delegator 'could not look up the latest mmhmem release.' }
if ($tag -notmatch '^v\d+\.\d+\.\d+$') { Stop-Delegator "the latest release tag '$tag' is not a stable version (vX.Y.Z)." }
try {
    $response = Invoke-WebRequest -UseBasicParsing -Uri "https://raw.githubusercontent.com/norandom/mmhmem/$tag/install.ps1"
    $text = [Text.Encoding]::UTF8.GetString($response.RawContentStream.ToArray()).TrimStart([char]0xFEFF)
} catch { Stop-Delegator "could not download the installer for release $tag." }
if (-not $text.Trim()) { Stop-Delegator "the installer for release $tag is empty." }
if ($text.Contains('mmhmem-transition-delegator')) { Stop-Delegator "release $tag ships the transition delegator itself; refusing to recurse." }
$text = $text.Replace('https://raw.githubusercontent.com/norandom/mmhmem/HEAD', "https://raw.githubusercontent.com/norandom/mmhmem/$tag")
$text = $text.Replace('https://github.com/norandom/mmhmem/releases/latest/download', "https://github.com/norandom/mmhmem/releases/download/$tag")
if ($text.Contains('/HEAD')) { Stop-Delegator "the installer for release $tag still refers to the moving HEAD; refusing." }
if ($text.Contains('releases/latest')) { Stop-Delegator "the installer for release $tag still refers to the latest release; refusing." }
& ([scriptblock]::Create($text)) @PSBoundParameters
