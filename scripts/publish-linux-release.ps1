#Requires -Version 5.1
<#
.SYNOPSIS
Deliver a verified CI-built static Linux bundle to an existing draft.
.DESCRIPTION
Only a named artifact from a trusted successful CI run is accepted. This
publisher never accepts a local binary or creates/publishes a release.
Use -ValidateOnly to check provenance and conflicts without uploading assets.
#>
[CmdletBinding()]
param([string]$Tag,[string]$RunId,[switch]$ValidateOnly)
. (Join-Path $PSScriptRoot 'linux-release-bundle.ps1')
$script:ReleaseRepository='norandom/mmhmem'
$script:ReleaseRepositoryRoot=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$script:ReleaseScratchRoot=Join-Path $script:ReleaseRepositoryRoot 'tmp'
function Invoke-ReleaseGh([string[]]$Arguments) {
    $previous=$ErrorActionPreference
    try {
        $ErrorActionPreference='Continue'; $global:LASTEXITCODE=1
        $output=& gh @Arguments 2>&1
        $code=$LASTEXITCODE
    } finally {$ErrorActionPreference=$previous}
    if ($code -ne 0) {throw "gh failed ($code): $($output -join "`n")"}
    return ($output -join "`n")
}
function Get-ReleaseApi([string]$Endpoint) {
    $raw=Invoke-ReleaseGh @('api','--method','GET',"repos/$($script:ReleaseRepository)/$Endpoint")
    return ($raw | ConvertFrom-Json)
}
function Get-ReleaseByTag([string]$ReleaseTag) {
    # The tag endpoint only returns published releases. Drafts are discoverable
    # through authenticated listings and must be refreshed by numeric ID.
    for ($page=1;$page -le 100;$page++) {
        $releases=@(Get-ReleaseApi "releases?per_page=100&page=$page")
        $matches=@($releases | Where-Object {$_.tag_name -ceq $ReleaseTag})
        if ($matches.Count -gt 1) {throw 'Duplicate release records for requested tag'}
        if ($matches.Count -eq 1) {
            $id=Get-ReleaseField $matches[0] 'id' integer
            if ($id -lt 1) {throw 'Invalid release ID'}
            $release=Get-ReleaseApi "releases/$id"
            if ($release.tag_name -cne $ReleaseTag -or (Get-ReleaseField $release 'id' integer) -ne $id) {throw 'Release identity changed during lookup'}
            return $release
        }
        if ($releases.Count -lt 100) {break}
    }
    throw 'Requested release not found in authenticated release listing'
}
function Resolve-ReleaseTag([string]$Tag) {
    $reference=Get-ReleaseApi ('git/ref/tags/'+[uri]::EscapeDataString($Tag))
    if ($reference.ref -cne ('refs/tags/'+$Tag)) {throw 'Requested tag reference mismatch'}
    $object=$reference.object
    for ($depth=0;$depth -lt 8;$depth++) {
        if ($object.sha -cnotmatch '^[0-9a-f]{40}$') {throw 'Tag has malformed commit identity'}
        if ($object.type -ceq 'commit') {return $object.sha}
        if ($object.type -cne 'tag') {throw 'Tag does not resolve to a commit'}
        $object=(Get-ReleaseApi ('git/tags/'+$object.sha)).object
    }
    throw 'Annotated tag chain exceeds supported depth'
}
function New-ReleaseScratch {
    $root=[IO.Path]::GetFullPath($script:ReleaseScratchRoot)
    $allowed=Join-Path $script:ReleaseRepositoryRoot 'tmp'
    if ($root -ne $allowed -and -not $root.StartsWith($allowed+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)) {throw 'Release scratch must remain in repository tmp/'}
    $cursor=$root
    while ($cursor.Length -gt $script:ReleaseRepositoryRoot.Length) {
        $item=Get-Item -LiteralPath $cursor -Force -ErrorAction SilentlyContinue
        if ($item -and ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {throw 'Release scratch path must not traverse a reparse point'}
        $cursor=Split-Path -Parent $cursor
    }
    $path=Join-Path $root ('linux-release-'+[guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $path -Force | Out-Null
    return $path
}
function Invoke-LinuxReleaseValidation {
    [CmdletBinding()]
    param([string]$Tag,[string]$RunId,[string[]]$SupersededAssets=@())
    if ($Tag -cnotmatch '^[A-Za-z0-9][A-Za-z0-9._/-]{0,127}$' -or $Tag.Contains('..') -or $Tag.EndsWith('/') -or $RunId -cnotmatch '^[1-9][0-9]{0,19}$') {throw 'Invalid Tag or RunId'}
    $revision=Resolve-ReleaseTag $Tag
    $run=Get-ReleaseApi "actions/runs/$RunId"
    if ($run.repository.full_name -cne $script:ReleaseRepository -or $run.head_repository.full_name -cne $script:ReleaseRepository) {throw 'Unexpected CI repository'}
    $workflow=Get-ReleaseApi 'actions/workflows/ci.yml'
    if ($run.name -cne 'CI' -or $run.path -cne '.github/workflows/ci.yml' -or $workflow.path -cne '.github/workflows/ci.yml' -or $workflow.state -cne 'active' -or (Get-ReleaseField $run 'workflow_id' integer) -ne (Get-ReleaseField $workflow 'id' integer)) {throw 'Unexpected CI workflow'}
    if ([string](Get-ReleaseField $run 'id' integer) -cne $RunId -or $run.status -cne 'completed' -or $run.conclusion -cne 'success') {throw 'CI run must be completed and successful'}
    if ($run.head_sha -cne $revision) {throw 'CI commit does not match requested tag commit'}
    if ($run.event -cne 'push' -or $run.head_branch -cne 'main' -or ($run.run_attempt -isnot [int] -and $run.run_attempt -isnot [long]) -or $run.run_attempt -lt 1) {throw 'CI run is not a trusted main-branch push/attempt'}
    $attempt=$run.run_attempt
    $jobs=Get-ReleaseApi "actions/runs/$RunId/attempts/$attempt/jobs?per_page=100"
    if ($jobs.total_count -ne @($jobs.jobs).Count) {throw 'Incomplete CI job listing'}
    foreach ($name in @('Blocking quality gates (Dagger)','Windows x64 - clippy, deterministic tests, release build','macOS aarch64 and x86_64 - clippy, deterministic tests, release builds')) {
        $matches=@($jobs.jobs | Where-Object {$_.name -ceq $name})
        if ($matches.Count -ne 1 -or $matches[0].status -cne 'completed' -or $matches[0].conclusion -cne 'success') {throw "Required CI job did not succeed: $name"}
    }
    $artifactName="linux-static-$revision-$RunId-$attempt"
    $artifacts=Get-ReleaseApi "actions/runs/$RunId/artifacts?per_page=100"
    if ($artifacts.total_count -ne @($artifacts.artifacts).Count) {throw 'Incomplete CI artifact listing'}
    $artifact=@($artifacts.artifacts | Where-Object {$_.name -ceq $artifactName})
    if ($artifact.Count -ne 1 -or $artifact[0].expired -isnot [bool] -or $artifact[0].expired) {throw 'Required uniquely named CI artifact missing or expired'}
    $scratch=New-ReleaseScratch
    $bundle=Join-Path $scratch 'bundle'
    New-Item -ItemType Directory -Path $bundle -Force | Out-Null
    $null=Invoke-ReleaseGh @('run','download',$RunId,'--repo',$script:ReleaseRepository,'--name',$artifactName,'--dir',$bundle)
    $manifest=Assert-LinuxReleaseBundle $bundle $revision "ci-$RunId-$attempt"
    $release=Get-ReleaseByTag $Tag
    if ($release.draft -isnot [bool] -or -not $release.draft -or $release.tag_name -cne $Tag -or ($release.id -isnot [int] -and $release.id -isnot [long]) -or $release.id -lt 1) {throw 'Destination must be the existing requested draft release'}
    $hashes=@{}; $existing=@{}
    foreach ($name in @($script:LinuxAssetName,'SHA256SUMS','verification.json')) {
        $hashes[$name]=(Get-FileHash -LiteralPath (Get-ReleaseBundleFile $bundle $name) -Algorithm SHA256).Hash.ToLowerInvariant()
        $assets=@($release.assets | Where-Object {$_.name -ceq $name})
        if ($assets.Count -gt 1) {throw "Conflicting duplicate Linux asset: $name"}
        if ($assets.Count -eq 1 -and $name -cnotin $SupersededAssets) {
            if ($assets[0].state -cne 'uploaded') {throw "Conflicting incomplete Linux asset: $name"}
            $remote=Join-Path $scratch ('remote-'+$name)
            New-Item -ItemType Directory -Path $remote | Out-Null
            $null=Invoke-ReleaseGh @('release','download',$Tag,'--repo',$script:ReleaseRepository,'--pattern',$name,'--dir',$remote)
            $actual=(Get-FileHash -LiteralPath (Get-ReleaseBundleFile $remote $name) -Algorithm SHA256).Hash.ToLowerInvariant()
            if ($actual -cne $hashes[$name]) {throw "Conflicting Linux release asset: $name"}
            $existing[$name]=$actual
        }
    }
    if ($hashes[$script:LinuxAssetName] -cne $manifest.artifact.sha256) {throw 'Artifact checksum changed after validation'}
    return [pscustomobject]@{Tag=$Tag;RunId=$RunId;RunAttempt=$attempt;Revision=$revision;ReleaseId=$release.id;ArtifactName=$artifactName;BundleDirectory=$bundle;ScratchDirectory=$scratch;Manifest=$manifest;Hashes=$hashes;ExistingHashes=$existing;SupersededAssets=$SupersededAssets}
}
function Assert-LinuxDeliveryFiles($Context) {
    foreach ($name in @($script:LinuxAssetName,'SHA256SUMS','verification.json')) {
        $actual=(Get-FileHash -LiteralPath (Get-ReleaseBundleFile $Context.BundleDirectory $name) -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actual -cne $Context.Hashes[$name]) {throw "Validated delivery file changed: $name"}
    }
}
function Get-LinuxDeliveryDraft($Context) {
    if ((Resolve-ReleaseTag $Context.Tag) -cne $Context.Revision) {throw 'Tag commit changed after validation'}
    $release=Get-ReleaseByTag $Context.Tag
    if ($release.draft -isnot [bool] -or -not $release.draft -or $release.tag_name -cne $Context.Tag -or
        (Get-ReleaseField $release 'id' integer) -ne $Context.ReleaseId) {throw 'Validated draft release state changed; refusing delivery'}
    return $release
}
function Get-LinuxDeliveryAsset($Release,[string]$Name) {
    $assets=@($Release.assets | Where-Object {$_.name -ceq $Name})
    if ($assets.Count -gt 1 -or ($assets.Count -eq 1 -and $assets[0].state -cne 'uploaded')) {throw "Conflicting duplicate or incomplete Linux asset: $Name"}
    if ($assets.Count) {return $assets[0]}
    return $null
}
function Assert-LinuxRemoteBytes($Context,[string]$Name) {
    $destination=Join-Path $Context.ScratchDirectory ('confirm-'+[guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $destination | Out-Null
    $null=Invoke-ReleaseGh @('release','download',$Context.Tag,'--repo',$script:ReleaseRepository,'--pattern',$Name,'--dir',$destination)
    $actual=(Get-FileHash -LiteralPath (Get-ReleaseBundleFile $destination $Name) -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -cne $Context.Hashes[$Name]) {throw "Conflicting or corrupted remote Linux asset: $Name"}
    return $actual
}
function Invoke-LinuxReleaseDelivery {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true)]$Context)
    Assert-LinuxDeliveryFiles $Context
    $delivered=@{}
    # SupersededAssets are validated locally but not delivered (CI replaces the
    # single-file SHA256SUMS with the manifest covering every release asset).
    foreach ($name in @($script:LinuxAssetName,'SHA256SUMS','verification.json')) {
        if ($name -cin @($Context.SupersededAssets)) {continue}
        $release=Get-LinuxDeliveryDraft $Context
        $existing=Get-LinuxDeliveryAsset $release $name
        if (-not $existing) {
            # Recheck every file immediately before each write. A mutation of a
            # later file must not silently reuse its prior validation result.
            Assert-LinuxDeliveryFiles $Context
            $path=Get-ReleaseBundleFile $Context.BundleDirectory $name
            $uploadFailure=$null
            try {$null=Invoke-ReleaseGh @('release','upload',$Context.Tag,$path,'--repo',$script:ReleaseRepository)}
            catch {$uploadFailure=$_.Exception.Message}
            # Upload can succeed remotely while its acknowledgement is lost.
            # Inspect remote state once; never overwrite or blindly retry.
            $release=Get-LinuxDeliveryDraft $Context
            if (-not (Get-LinuxDeliveryAsset $release $name)) {
                throw "Upload did not produce a completed asset: $name. $uploadFailure"
            }
        }
        $delivered[$name]=Assert-LinuxRemoteBytes $Context $name
    }
    Assert-LinuxDeliveryFiles $Context
    $null=Get-LinuxDeliveryDraft $Context
    return [pscustomobject]@{Tag=$Context.Tag;ReleaseId=$Context.ReleaseId;Revision=$Context.Revision;Hashes=$delivered}
}
function Invoke-LinuxReleasePublish {
    [CmdletBinding()]
    param([string]$Tag,[string]$RunId,[switch]$ValidateOnly)
    $context=Invoke-LinuxReleaseValidation -Tag $Tag -RunId $RunId
    if ($ValidateOnly) {return $context}
    return (Invoke-LinuxReleaseDelivery $context)
}
if ($MyInvocation.InvocationName -ne '.') {
    $ErrorActionPreference='Stop'
    $result=Invoke-LinuxReleasePublish -Tag $Tag -RunId $RunId -ValidateOnly:$ValidateOnly
    if ($ValidateOnly) {Write-Host "Validated draft inputs: tag=$Tag run=$RunId revision=$($result.Revision). No assets uploaded."}
    else {Write-Host "Verified draft delivery: tag=$Tag revision=$($result.Revision) sha256=$($result.Hashes[$script:LinuxAssetName]). Release remains a draft."}
}
