#Requires -Version 5.1
<#
Publish version tags from the successful, trusted main CI run's artifacts.
The Linux publisher retains its full provenance and byte-equality checks.
#>
[CmdletBinding()]
param([string]$Tag,[string]$RunId)
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
function Invoke-CiRelease([string]$ReleaseTag,[string]$RequestedRun) {
    if (-not $ReleaseTag) {
        if ($RequestedRun -cnotmatch '^[1-9][0-9]*$') {throw 'Automatic completion requires a run ID'}
        $run=Get-ReleaseApi "actions/runs/$RequestedRun"
        Assert-CiReleaseRun $run $run.head_sha
        $tags=@(& git tag --points-at $run.head_sha)
        if ($LASTEXITCODE -ne 0) {throw 'Cannot inspect tags'}
        $tags=@($tags | Where-Object {$_ -cmatch '^v[0-9]+\.[0-9]+\.[0-9]+$'})
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
    $id=[string]$run.id
    foreach ($entry in @(@('windows','mmhmem-x86_64-pc-windows-msvc'),@('macos','mmhmem-macos'))) {
        $destination=Join-Path $scratch $entry[0]
        $null=Invoke-ReleaseGh @('run','download',$id,'--repo',$script:ReleaseRepository,'--name',$entry[1],'--dir',$destination)
    }
    $native=Get-CiNativeAssets $scratch
    $releases=Get-ReleaseApi 'releases?per_page=100'
    $existing=@($releases | Where-Object {$_.tag_name -ceq $ReleaseTag})
    if ($existing.Count -gt 1) {throw 'Duplicate release records'}
    if ($existing.Count -and -not $existing[0].draft) {Write-Host "$ReleaseTag is already published; no assets changed.";return}
    if (-not $existing.Count) {$null=Invoke-ReleaseGh @('release','create',$ReleaseTag,'--repo',$script:ReleaseRepository,'--verify-tag','--draft','--title',"mmhmem $ReleaseTag",'--generate-notes')}
    $context=Invoke-LinuxReleaseValidation -Tag $ReleaseTag -RunId $id -SupersededAssets @('SHA256SUMS')
    if ($context.Manifest.package_version -cne $ReleaseTag.Substring(1)) {throw 'Tag does not match verified binary package version'}
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
    $null=Get-LinuxDeliveryDraft $context
    if ((Resolve-ReleaseTag $ReleaseTag) -cne $revision) {throw 'Tag changed before publication'}
    $null=Invoke-ReleaseGh @('release','edit',$ReleaseTag,'--repo',$script:ReleaseRepository,'--draft=false','--latest')
    } finally {Close-ReleaseSigning $signing}
    Write-Host "Published $ReleaseTag from successful CI run $id ($revision)"
}
if ($MyInvocation.InvocationName -ne '.') {
    $ErrorActionPreference='Stop'
    Invoke-CiRelease $Tag $RunId
}
