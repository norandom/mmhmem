#Requires -Version 5.1
<#
Publish version tags from the successful, trusted main CI run's artifacts.
The Linux publisher retains its full provenance and byte-equality checks.
A release needs green main CI AND a green installer e2e gate for the exact
tag commit (Requirement 10.5). It is built as a draft; the workflow attests
the archives and only then calls this script again with -PublishDraft, which
re-checks the evidence and the draft and publishes it as the last step.
#>
[CmdletBinding()]
param([string]$Tag,[string]$RunId,[string]$DraftRevision,[string]$DraftAssets,[switch]$PublishDraft)
. (Join-Path $PSScriptRoot 'publish-linux-release.ps1') -Tag $Tag -RunId $RunId
. (Join-Path $PSScriptRoot 'package-release-archives.ps1')
. (Join-Path $PSScriptRoot 'sign-release-assets.ps1')
. (Join-Path $PSScriptRoot 'render-bootstraps.ps1')

function Assert-CiReleaseTag([string]$Value) {
    if ($Value -cnotmatch '^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$') {throw 'Only stable vMAJOR.MINOR.PATCH tags can be published'}
}
function Get-CiReleaseRun([string]$Revision,[string]$RequestedRun) {
    if ($RequestedRun) {
        if ($RequestedRun -cnotmatch '^[1-9][0-9]*$') {throw 'Invalid run ID'}
        return Get-ReleaseApi "actions/runs/$RequestedRun"
    }
    # A tag may be pushed before its main-branch CI finishes. Waiting here
    # also handles tags pushed after CI, without rebuilding tagged sources.
    for ($attempt=0;$attempt -lt 90;$attempt++) {
        $runs=Get-ReleaseApi "actions/workflows/ci.yml/runs?branch=main&event=push&head_sha=$Revision&per_page=100"
        $matches=@($runs.workflow_runs | Where-Object {$_.head_sha -ceq $Revision -and $_.event -ceq 'push' -and $_.head_branch -ceq 'main'})
        if ($matches.Count) {
            $run=$matches | Sort-Object id -Descending | Select-Object -First 1
            if ($run.status -ceq 'completed') {return $run}
        }
        Start-Sleep -Seconds 20
    }
    throw 'No completed main CI run for the tagged commit within 30 minutes'
}
function Assert-CiReleaseRun($Run,[string]$Revision) {
    if ($Run.head_sha -cne $Revision -or $Run.event -cne 'push' -or $Run.head_branch -cne 'main' -or
        $Run.status -cne 'completed' -or $Run.conclusion -cne 'success' -or $Run.path -cne '.github/workflows/ci.yml' -or
        $Run.repository.full_name -cne $script:ReleaseRepository -or $Run.head_repository.full_name -cne $script:ReleaseRepository) {throw 'Release requires successful trusted main CI for the exact tag commit'}
}
# Installer e2e gate (task 8.3, Requirement 10.5). Only a push run on main of
# installer-e2e.yml counts: a workflow_dispatch run (the only kind that can
# carry sabotage=wrong-hash) or a pull_request run is never evidence. The
# newest push run for the commit decides, so an older green run cannot hide a
# newer red one, and its aggregate job `installer-e2e` must have passed.
$script:InstallerE2EWorkflow='.github/workflows/installer-e2e.yml'
$script:InstallerE2EGateJob='installer-e2e'
# 90 x 20 s: installer e2e starts with CI on the same push and may end later.
$script:InstallerE2EWaitAttempts=90
function Get-InstallerE2ERun([string]$Revision,[int]$Attempts=$script:InstallerE2EWaitAttempts) {
    if ($Revision -cnotmatch '^[0-9a-f]{40}$') {throw 'Invalid revision'}
    for ($attempt=0;$attempt -lt $Attempts;$attempt++) {
        if ($attempt) {Start-Sleep -Seconds 20}
        $runs=Get-ReleaseApi "actions/workflows/installer-e2e.yml/runs?branch=main&event=push&head_sha=$Revision&per_page=100"
        $matches=@($runs.workflow_runs | Where-Object {$_.head_sha -ceq $Revision -and $_.event -ceq 'push' -and $_.head_branch -ceq 'main'})
        if ($matches.Count) {
            $run=$matches | Sort-Object id -Descending | Select-Object -First 1
            if ($run.status -ceq 'completed') {return $run}
        }
    }
    throw 'Release requires a completed installer e2e run (push to main) for the exact tag commit; none finished in time'
}
function Assert-InstallerE2ERun($Run,[string]$Revision) {
    if ($Run.head_sha -cne $Revision -or $Run.event -cne 'push' -or $Run.head_branch -cne 'main' -or
        $Run.status -cne 'completed' -or $Run.conclusion -cne 'success' -or $Run.path -cne $script:InstallerE2EWorkflow -or
        $Run.repository.full_name -cne $script:ReleaseRepository -or $Run.head_repository.full_name -cne $script:ReleaseRepository -or
        [string]$Run.id -cnotmatch '^[1-9][0-9]*$') {throw 'Release requires a green installer e2e run (push to main) for the exact tag commit'}
    $jobs=@((Get-ReleaseApi "actions/runs/$($Run.id)/jobs?filter=latest&per_page=100").jobs | Where-Object {$_.name -ceq $script:InstallerE2EGateJob})
    if ($jobs.Count -ne 1 -or $jobs[0].status -cne 'completed' -or $jobs[0].conclusion -cne 'success') {throw "Release requires the $($script:InstallerE2EGateJob) job to pass in installer e2e run $($Run.id)"}
}
# The draft must hold exactly the verified assets, all fully uploaded:
# anything else would be published without having been checked.
function Assert-CiDraftAssets($Release,[string[]]$Expected) {
    $names=[string[]]@($Release.assets | Where-Object {$null -ne $_} | ForEach-Object {
        if ($_.state -cne 'uploaded') {throw "Incomplete release asset: $($_.name)"}
        [string]$_.name
    })
    $want=[string[]]@($Expected)
    [Array]::Sort($names,[StringComparer]::Ordinal)
    [Array]::Sort($want,[StringComparer]::Ordinal)
    if (($names -join ',') -cne ($want -join ',')) {throw "Draft assets differ from the verified set: draft [$($names -join ',')], verified [$($want -join ',')]"}
}
function Get-CiNativeAssets([string]$Root) {
    $windows=@(Get-ChildItem -LiteralPath (Join-Path $Root 'windows') -File -Recurse)
    $mac=@(Get-ChildItem -LiteralPath (Join-Path $Root 'macos') -File -Recurse)
    if ($windows.Count -ne 1 -or $windows[0].Name -cne 'mmhmem.exe' -or $mac.Count -ne 2) {throw 'Unexpected native artifact layout'}
    $intel=@($mac | Where-Object {$_.FullName -match '[\\/]x86_64-apple-darwin[\\/]release[\\/]mmhmem$'})
    $arm=@($mac | Where-Object {$_.FullName -notmatch '[\\/]x86_64-apple-darwin[\\/]' -and $_.Name -ceq 'mmhmem'})
    if ($intel.Count -ne 1 -or $arm.Count -ne 1) {throw 'Missing unique macOS architecture binaries'}
    foreach ($file in @($windows[0],$intel[0],$arm[0])) {
        if ($file.Length -eq 0 -or ($file.Attributes -band [IO.FileAttributes]::ReparsePoint)) {throw 'Invalid native binary'}
    }
    return @{'mmhmem.exe'=$windows[0].FullName;'mmhmem-macos-x86_64'=$intel[0].FullName;'mmhmem-macos-aarch64'=$arm[0].FullName}
}
# A binary built with the installer e2e feature (`e2e-release-key`) trusts a
# throwaway key and must never be published; such a build carries this
# marker (src/setup/release_keys.rs, E2E_BUILD_MARKER).
$script:E2EBuildMarker='mmhmem-e2e-release-key-build'
function Assert-NoE2EBuildMarker([string]$Path) {
    $latin1=[Text.Encoding]::GetEncoding(28591)
    if ($latin1.GetString([IO.File]::ReadAllBytes($Path)).IndexOf($script:E2EBuildMarker,[StringComparison]::Ordinal) -ge 0) {
        throw "Refusing to publish $(Split-Path -Leaf $Path): it is an installer e2e build (e2e-release-key) that trusts a test key"
    }
}
function Send-CiReleaseAsset([string]$ReleaseTag,[string]$Path,[string]$Scratch) {
    $name=Split-Path -Leaf $Path
    $expected=(Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
    $release=Get-ReleaseByTag $ReleaseTag
    if (-not $release.draft) {throw 'Refusing to modify a published release'}
    $assets=@($release.assets | Where-Object {$_.name -ceq $name})
    if ($assets.Count -gt 1) {throw 'Duplicate release asset'}
    if (-not $assets.Count) {$null=Invoke-ReleaseGh @('release','upload',$ReleaseTag,$Path,'--repo',$script:ReleaseRepository)}
    $remote=Join-Path $Scratch ([guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $remote | Out-Null
    $null=Invoke-ReleaseGh @('release','download',$ReleaseTag,'--repo',$script:ReleaseRepository,'--pattern',$name,'--dir',$remote)
    if ((Get-FileHash -LiteralPath (Join-Path $remote $name) -Algorithm SHA256).Hash -cne $expected) {throw "Remote bytes differ: $name"}
}
# Stage every published asset except SHA256SUMS in one directory: bare
# transition binaries, verification.json and the four per-target archives.
# Returns the staged names; hashing is a separate, re-runnable step.
function Initialize-CiReleaseAssets([string]$ReleaseTag,[hashtable]$Native,[string]$LinuxBundle,[string]$Staging) {
    New-Item -ItemType Directory -Path $Staging -Force | Out-Null
    $linux=Get-ReleaseBundleFile $LinuxBundle $script:LinuxAssetName
    $bare=@{'mmhmem.exe'=$Native['mmhmem.exe'];'mmhmem-macos-x86_64'=$Native['mmhmem-macos-x86_64'];'mmhmem-macos-aarch64'=$Native['mmhmem-macos-aarch64'];($script:LinuxAssetName)=$linux;'verification.json'=(Get-ReleaseBundleFile $LinuxBundle 'verification.json')}
    foreach ($binary in @($Native['mmhmem.exe'],$Native['mmhmem-macos-x86_64'],$Native['mmhmem-macos-aarch64'],$linux)) {Assert-NoE2EBuildMarker $binary}
    foreach ($name in $bare.Keys) {Copy-Item -LiteralPath $bare[$name] -Destination (Join-Path $Staging $name)}
    $archives=New-ReleaseArchives $ReleaseTag @{'windows'=$Native['mmhmem.exe'];'linux'=$linux;'macos-x86_64'=$Native['mmhmem-macos-x86_64'];'macos-aarch64'=$Native['mmhmem-macos-aarch64']} $Staging
    $names=[string[]]@(@($bare.Keys)+@($archives | ForEach-Object {Split-Path -Leaf $_}))
    [Array]::Sort($names,[StringComparer]::Ordinal)
    return ,$names
}
# Render install.sh / install.ps1 into the staging directory from the signed
# archives and return the extended, sorted asset-name list. Must run after
# archive signing and before Write-ReleaseChecksums.
function Add-CiBootstrapAssets([string]$ReleaseTag,[string]$Staging,[string[]]$Names) {
    $scripts=Write-RenderedBootstraps $ReleaseTag $Staging $Staging
    $all=[string[]]@(@($Names)+@($scripts) | Select-Object -Unique)
    [Array]::Sort($all,[StringComparer]::Ordinal)
    return ,$all
}
function Write-AttestationSubjects([string[]]$Paths) {
    if (-not $env:GITHUB_OUTPUT) {return}
    $list=($Paths | ForEach-Object {$_.Replace('\','/')}) -join "`n"
    [IO.File]::AppendAllText($env:GITHUB_OUTPUT,"archives<<MMHMEM_ARCHIVES`n$list`nMMHMEM_ARCHIVES`n",(New-Object Text.UTF8Encoding($false)))
}
# Handed to the attestation and -PublishDraft steps of the workflow.
function Write-CiDraftOutputs([string]$ReleaseTag,[string]$Revision,[string]$RunId,[string[]]$Names) {
    if (-not $env:GITHUB_OUTPUT) {return}
    $text="tag=$ReleaseTag`nrevision=$Revision`nci_run=$RunId`nassets=$($Names -join ',')`n"
    [IO.File]::AppendAllText($env:GITHUB_OUTPUT,$text,(New-Object Text.UTF8Encoding($false)))
}
# The release page starts with the install one-liners (.github/release-notes.md,
# task 9.1); GitHub's generated notes follow them.
$script:ReleaseNotesFile=Join-Path (Split-Path -Parent $PSScriptRoot) '.github/release-notes.md'
function Get-CiReleaseCreateArguments([string]$ReleaseTag) {
    if (-not (Test-Path -LiteralPath $script:ReleaseNotesFile -PathType Leaf)) {throw "Missing release notes template: $($script:ReleaseNotesFile)"}
    return [string[]]@('release','create',$ReleaseTag,'--repo',$script:ReleaseRepository,'--verify-tag','--draft','--title',"mmhmem $ReleaseTag",'--notes-file',$script:ReleaseNotesFile,'--generate-notes')
}
function Invoke-CiRelease([string]$ReleaseTag,[string]$RequestedRun) {
    if (-not $ReleaseTag) {
        if ($RequestedRun -cnotmatch '^[1-9][0-9]*$') {throw 'Automatic completion requires a run ID'}
        $run=Get-ReleaseApi "actions/runs/$RequestedRun"
        Assert-CiReleaseRun $run $run.head_sha
        $tags=@(& git tag --points-at $run.head_sha)
        if ($LASTEXITCODE -ne 0) {throw 'Cannot inspect tags'}
        $tags=@($tags | Where-Object {$_ -cmatch '^v[0-9]+\.[0-9]+\.[0-9]+$'})
        # One draft per job: the attestation and publish steps take one tag.
        if ($tags.Count -gt 1) {throw 'The commit has more than one version tag; publish each with workflow_dispatch'}
        foreach ($candidate in $tags) {Invoke-CiRelease $candidate $RequestedRun}
        if (-not $tags.Count) {Write-Host 'Successful CI commit has no version tag; nothing to publish.'}
        return
    }
    Assert-CiReleaseTag $ReleaseTag
    $scratch=New-ReleaseScratch
    # Fail closed before any GitHub read or write: no signing key, no release.
    $signing=Open-ReleaseSigning (Join-Path $scratch 'signing')
    try {
    $revision=Resolve-ReleaseTag $ReleaseTag
    $run=Get-CiReleaseRun $revision $RequestedRun
    Assert-CiReleaseRun $run $revision
    # Requirement 10.5: no artifact download, no draft and no upload without
    # a green installer e2e run for the same commit.
    Assert-InstallerE2ERun (Get-InstallerE2ERun $revision) $revision
    $id=[string]$run.id
    foreach ($entry in @(@('windows','mmhmem-x86_64-pc-windows-msvc'),@('macos','mmhmem-macos'))) {
        $destination=Join-Path $scratch $entry[0]
        $null=Invoke-ReleaseGh @('run','download',$id,'--repo',$script:ReleaseRepository,'--name',$entry[1],'--dir',$destination)
    }
    $native=Get-CiNativeAssets $scratch
    foreach ($binary in $native.Values) {Assert-NoE2EBuildMarker $binary}
    $releases=Get-ReleaseApi 'releases?per_page=100'
    $existing=@($releases | Where-Object {$_.tag_name -ceq $ReleaseTag})
    if ($existing.Count -gt 1) {throw 'Duplicate release records'}
    if ($existing.Count -and -not $existing[0].draft) {Write-Host "$ReleaseTag is already published; no assets changed.";return}
    if (-not $existing.Count) {$null=Invoke-ReleaseGh (Get-CiReleaseCreateArguments $ReleaseTag)}
    $context=Invoke-LinuxReleaseValidation -Tag $ReleaseTag -RunId $id -SupersededAssets @('SHA256SUMS')
    if ($context.Manifest.package_version -cne $ReleaseTag.Substring(1)) {throw 'Tag does not match verified binary package version'}
    Assert-NoE2EBuildMarker (Get-ReleaseBundleFile $context.BundleDirectory $script:LinuxAssetName)
    $null=Invoke-LinuxReleaseDelivery $context
    $staging=Join-Path $scratch 'assets'
    $staged=Initialize-CiReleaseAssets $ReleaseTag $native $context.BundleDirectory $staging
    # Sign the archives (context = final file name), THEN hash every asset, then
    # sign SHA256SUMS. All signing finishes before the remaining uploads.
    $signedArchives=@($staged | Where-Object {$_ -cmatch '\.(zip|tar\.gz)$'} | ForEach-Object {Join-Path $staging $_})
    Invoke-ReleaseSigning $signedArchives $signing
    # The bootstraps embed the hashes of the final signed archives.
    $staged=Add-CiBootstrapAssets $ReleaseTag $staging $staged
    $sumPath=Write-ReleaseChecksums $staging $staged
    $sigPath=Write-ReleaseChecksumSignature $staging $signing
    # Hand the final signed archives to the workflow's provenance attestation step.
    Write-AttestationSubjects $signedArchives
    # Everything except the Linux files already delivered above is uploaded and
    # re-downloaded byte for byte; SHA256SUMS then SHA256SUMS.sig go last.
    foreach ($name in $staged) {
        if ($name -cin @($script:LinuxAssetName,'verification.json')) {continue}
        Send-CiReleaseAsset $ReleaseTag (Join-Path $staging $name) $scratch
    }
    Send-CiReleaseAsset $ReleaseTag $sumPath $scratch
    Send-CiReleaseAsset $ReleaseTag $sigPath $scratch
    Assert-LinuxDeliveryFiles $context
    $verified=[string[]]@(@($staged)+@('SHA256SUMS','SHA256SUMS.sig'))
    Assert-CiDraftAssets (Get-LinuxDeliveryDraft $context) $verified
    if ((Resolve-ReleaseTag $ReleaseTag) -cne $revision) {throw 'Tag changed before publication'}
    # The release stays a draft: the workflow attests the archives, then
    # publishes with -PublishDraft as its last step.
    Write-CiDraftOutputs $ReleaseTag $revision $id $verified
    } finally {Close-ReleaseSigning $signing}
    Write-Host "Verified draft $ReleaseTag from successful CI run $id ($revision); it is published after attestation"
}
# Last step of the workflow, after the attestation; never sees the signing key.
# Re-reads the evidence (a re-run can turn a green run red) and the draft.
function Publish-CiReleaseDraft([string]$ReleaseTag,[string]$RequestedRun,[string]$Revision,[string]$AssetList) {
    Assert-CiReleaseTag $ReleaseTag
    if ($Revision -cnotmatch '^[0-9a-f]{40}$') {throw 'Invalid revision'}
    if ($RequestedRun -cnotmatch '^[1-9][0-9]*$') {throw 'Invalid run ID'}
    $expected=[string[]]@($AssetList -split ',' | Where-Object {$_})
    if (-not $expected.Count) {throw 'No verified asset list'}
    if ((Resolve-ReleaseTag $ReleaseTag) -cne $Revision) {throw 'Tag changed before publication'}
    Assert-CiReleaseRun (Get-ReleaseApi "actions/runs/$RequestedRun") $Revision
    Assert-InstallerE2ERun (Get-InstallerE2ERun $Revision 1) $Revision
    $release=Get-ReleaseByTag $ReleaseTag
    if ($release.draft -isnot [bool] -or -not $release.draft) {throw 'Refusing to publish: the release is no longer a draft'}
    Assert-CiDraftAssets $release $expected
    $null=Invoke-ReleaseGh @('release','edit',$ReleaseTag,'--repo',$script:ReleaseRepository,'--draft=false','--latest')
    Write-Host "Published $ReleaseTag ($Revision) from successful CI run $RequestedRun"
}
if ($MyInvocation.InvocationName -ne '.') {
    $ErrorActionPreference='Stop'
    if ($PublishDraft) {Publish-CiReleaseDraft $Tag $RunId $DraftRevision $DraftAssets}
    else {Invoke-CiRelease $Tag $RunId}
}
