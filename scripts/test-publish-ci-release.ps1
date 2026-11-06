#Requires -Version 5.1
# Offline regression checks. No GitHub writes or executable fixtures.
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'publish-ci-release.ps1') -Tag 'v0.3.0' -RunId '123'
if ($Tag -cne 'v0.3.0' -or $RunId -cne '123') {throw 'Entrypoint parameters were lost while loading the Linux publisher'}
$scratch=New-ReleaseScratch
function Assert-Rejected([scriptblock]$Action) {
    $rejected=$false
    try {& $Action} catch {$rejected=$true}
    if (-not $rejected) {throw 'Invalid release input was accepted'}
}
Assert-CiReleaseTag 'v0.3.0'
foreach ($bad in @('main','v0.3','v01.3.0','v0.3.0-rc1','v0.3.0;echo bad')) {Assert-Rejected {Assert-CiReleaseTag $bad}}
$revision='a'*40
$validRun=@{id=123;head_sha=$revision;event='push';head_branch='main';status='completed';conclusion='success';path='.github/workflows/ci.yml';repository=@{full_name='norandom/mmhmem'};head_repository=@{full_name='norandom/mmhmem'}}
Assert-CiReleaseRun $validRun $revision
foreach ($change in @(@('event','pull_request'),@('head_sha',('b'*40)),@('head_branch','feature'),@('status','in_progress'),@('conclusion','failure'),@('path','.github/workflows/other.yml'))) {
    $bad=$validRun.Clone();$bad[$change[0]]=$change[1]
    Assert-Rejected {Assert-CiReleaseRun $bad $revision}
}
$bad=$validRun.Clone();$bad.head_repository=@{full_name='fork/mmhmem'}
Assert-Rejected {Assert-CiReleaseRun $bad $revision}
foreach ($relative in @('windows','macos/release','macos/x86_64-apple-darwin/release')) {New-Item -ItemType Directory -Path (Join-Path $scratch $relative) -Force|Out-Null}
foreach ($relative in @('windows/mmhmem.exe','macos/release/mmhmem','macos/x86_64-apple-darwin/release/mmhmem')) {[IO.File]::WriteAllText((Join-Path $scratch $relative),'synthetic fixture')}
$assets=Get-CiNativeAssets $scratch
if ($assets.Count -ne 3 -or $assets['mmhmem-macos-aarch64'] -notmatch '[\\/]macos[\\/]release[\\/]mmhmem$') {throw 'Native mapping mismatch'}
[IO.File]::WriteAllText((Join-Path $scratch 'windows/extra.exe'),'extra')
Assert-Rejected {Get-CiNativeAssets $scratch}
Remove-Item -LiteralPath (Join-Path $scratch 'windows/extra.exe')
[IO.File]::WriteAllText((Join-Path $scratch 'windows/mmhmem.exe'),'')
Assert-Rejected {Get-CiNativeAssets $scratch}
$script:UploadCount=0
$script:RemoteContent='expected asset bytes'
$script:Published=$false
$script:HasAsset=$false
function Get-ReleaseApi([string]$Endpoint) {
    if ($Endpoint -like 'releases/tags/*') {throw 'HTTP 404: tag endpoint excludes drafts'}
    if ($Endpoint -like 'releases?per_page=*') {return @([pscustomobject]@{id=456;tag_name='v0.3.0'})}
    if ($Endpoint -cne 'releases/456') {throw 'Unexpected release endpoint'}
    return [pscustomobject]@{id=456;tag_name='v0.3.0';draft=(-not $script:Published);assets=$(if($script:HasAsset){@([pscustomobject]@{name='native.bin'})}else{@()})}
}
function Invoke-ReleaseGh([string[]]$Arguments) {
    if ($Arguments[0] -ne 'release') {throw 'Unexpected mocked command'}
    if ($Arguments[1] -eq 'upload') {$script:UploadCount++;return ''}
    if ($Arguments[1] -eq 'download') {
        $destination=$Arguments[($Arguments.IndexOf('--dir')+1)]
        [IO.File]::WriteAllText((Join-Path $destination 'native.bin'),$script:RemoteContent)
        return ''
    }
    throw 'Unexpected mocked mutation'
}
$path=Join-Path $scratch 'native.bin';[IO.File]::WriteAllText($path,'expected asset bytes')
Send-CiReleaseAsset 'v0.3.0' $path $scratch
if ($script:UploadCount -ne 1) {throw 'Missing asset was not uploaded'}
$script:HasAsset=$true
Send-CiReleaseAsset 'v0.3.0' $path $scratch
if ($script:UploadCount -ne 1) {throw 'Existing identical asset was replaced'}
$script:RemoteContent='conflicting bytes'
Assert-Rejected {Send-CiReleaseAsset 'v0.3.0' $path $scratch}
$script:Published=$true
Assert-Rejected {Send-CiReleaseAsset 'v0.3.0' $path $scratch}
if ($script:UploadCount -ne 1) {throw 'Conflict caused an upload'}
# Staged release assets: archives, bare transition binaries, verification.json.
$bundle=Join-Path $scratch 'bundle';New-Item -ItemType Directory -Path $bundle | Out-Null
[IO.File]::WriteAllText((Join-Path $bundle $script:LinuxAssetName),'linux fixture');[IO.File]::WriteAllText((Join-Path $bundle 'verification.json'),'{}')
foreach ($relative in @('windows/mmhmem.exe')) {[IO.File]::WriteAllText((Join-Path $scratch $relative),'synthetic fixture')}
$native=Get-CiNativeAssets $scratch
$staging=Join-Path $scratch 'assets'
$staged=Initialize-CiReleaseAssets 'v0.3.0' $native $bundle $staging
$wanted='mmhmem-linux-x86_64,mmhmem-macos-aarch64,mmhmem-macos-x86_64,mmhmem-v0.3.0-aarch64-apple-darwin.tar.gz,mmhmem-v0.3.0-x86_64-apple-darwin.tar.gz,mmhmem-v0.3.0-x86_64-pc-windows-msvc.zip,mmhmem-v0.3.0-x86_64-unknown-linux-musl.tar.gz,mmhmem.exe,verification.json'
if (($staged -join ',') -cne $wanted) {throw "Unexpected staged asset set: $($staged -join ',')"}
$sumLines=@([IO.File]::ReadAllText((Write-ReleaseChecksums $staging $staged)).TrimEnd("`n") -split "`n")
if ($sumLines.Count -ne 9) {throw 'SHA256SUMS must list every staged asset'}
foreach ($name in $staged) {
    if ($sumLines -cnotcontains ((Get-FileHash -LiteralPath (Join-Path $staging $name) -Algorithm SHA256).Hash.ToLowerInvariant()+'  '+$name)) {throw "SHA256SUMS missing $name"}
}
# Bootstraps are rendered from the staged (signed) archives and covered by SHA256SUMS.
$staged=Add-CiBootstrapAssets 'v0.3.0' $staging $staged
if ($staged.Count -ne 11 -or $staged -cnotcontains 'install.sh' -or $staged -cnotcontains 'install.ps1') {throw 'Bootstrap scripts were not staged'}
$sumLines=@([IO.File]::ReadAllText((Write-ReleaseChecksums $staging $staged)).TrimEnd("`n") -split "`n")
if ($sumLines.Count -ne 11) {throw 'SHA256SUMS must list the bootstrap scripts'}
foreach ($name in @('install.sh','install.ps1')) {
    if ($sumLines -cnotcontains ((Get-FileHash -LiteralPath (Join-Path $staging $name) -Algorithm SHA256).Hash.ToLowerInvariant()+'  '+$name)) {throw "SHA256SUMS missing $name"}
}
# An installer e2e build (feature e2e-release-key) is never published.
Assert-NoE2EBuildMarker (Join-Path $staging 'mmhmem.exe')
$e2eBinary=Join-Path $scratch 'windows/mmhmem.exe'
[IO.File]::WriteAllBytes($e2eBinary,[byte[]](@(0,1,2)+[Text.Encoding]::ASCII.GetBytes('mmhmem-e2e-release-key-build')+@(255)))
Assert-Rejected {Assert-NoE2EBuildMarker $e2eBinary}
Assert-Rejected {Initialize-CiReleaseAssets 'v0.3.0' (Get-CiNativeAssets $scratch) $bundle (Join-Path $scratch 'assets-e2e')}
[IO.File]::WriteAllText($e2eBinary,'synthetic fixture')
$winHash=(Get-FileHash -LiteralPath (Join-Path $staging 'mmhmem-v0.3.0-x86_64-pc-windows-msvc.zip') -Algorithm SHA256).Hash.ToLowerInvariant()
if (-not [IO.File]::ReadAllText((Join-Path $staging 'install.ps1')).Contains($winHash)) {throw 'install.ps1 does not embed the staged archive hash'}
# Installer e2e gate (task 8.3, Requirement 10.5): a release needs the newest
# push run of installer-e2e.yml on main for the exact commit, green, with its
# aggregate job `installer-e2e` green. Fixtures are API-shaped objects.
function New-E2ERun([long]$Id,[string]$Event='push',[string]$Conclusion='success',[string]$Status='completed') {
    return [pscustomobject]@{id=$Id;head_sha=$revision;event=$Event;head_branch='main';status=$Status;conclusion=$Conclusion;path='.github/workflows/installer-e2e.yml';repository=[pscustomobject]@{full_name='norandom/mmhmem'};head_repository=[pscustomobject]@{full_name='norandom/mmhmem'}}
}
function New-GateJob([string]$Conclusion='success',[string]$Name='installer-e2e') {return [pscustomobject]@{name=$Name;status='completed';conclusion=$Conclusion}}
function Assert-RejectedWith([scriptblock]$Action,[string]$Expected) {
    $message=$null
    try {& $Action} catch {$message=$_.Exception.Message}
    if ($null -eq $message) {throw "Accepted, expected refusal: $Expected"}
    if (-not $message.Contains($Expected)) {throw "Refused for the wrong reason: $message (expected: $Expected)"}
}
$script:E2ERuns=@(New-E2ERun 789)
$script:E2EJobs=@((New-GateJob),[pscustomobject]@{name='e2e windows-pwsh-7';status='completed';conclusion='success'})
$script:CiRun=[pscustomobject]$validRun
$script:ApiCalls=New-Object System.Collections.Generic.List[string]
$script:GhCalls=New-Object System.Collections.Generic.List[string]
$script:MockDraftAssets=@()
$script:DraftState=$true
function Get-ReleaseApi([string]$Endpoint) {
    $script:ApiCalls.Add($Endpoint)
    if ($Endpoint -like 'actions/workflows/installer-e2e.yml/runs?*') {return [pscustomobject]@{workflow_runs=@($script:E2ERuns)}}
    if ($Endpoint -like 'actions/runs/*/jobs?*') {return [pscustomobject]@{jobs=@($script:E2EJobs)}}
    if ($Endpoint -ceq 'actions/runs/123') {return $script:CiRun}
    if ($Endpoint -like 'releases?per_page=100*') {return @([pscustomobject]@{id=456;tag_name='v0.3.0'})}
    if ($Endpoint -ceq 'releases/456') {return [pscustomobject]@{id=456;tag_name='v0.3.0';draft=$script:DraftState;assets=@($script:MockDraftAssets)}}
    throw "Unexpected endpoint: $Endpoint"
}
function Invoke-ReleaseGh([string[]]$Arguments) {
    $script:GhCalls.Add(($Arguments -join ' '))
    if ($Arguments[0] -ceq 'run' -and $Arguments[1] -ceq 'download') {throw 'MOCK: reached the artifact download'}
    if ($Arguments[0] -ceq 'release' -and $Arguments[1] -ceq 'edit') {return ''}
    throw "Unexpected mocked command: $($Arguments -join ' ')"
}
$script:TagRevision=$revision
function Resolve-ReleaseTag([string]$Tag) {return $script:TagRevision}
function Open-ReleaseSigning([string]$Directory) {return [pscustomobject]@{KeyFile=$null}}
function Close-ReleaseSigning($State) {}
$script:InstallerE2EWaitAttempts=1
# The run object itself.
$good=New-E2ERun 789
Assert-InstallerE2ERun $good $revision
foreach ($change in @(@('event','workflow_dispatch'),@('event','pull_request'),@('event','workflow_call'),@('head_sha',('b'*40)),@('head_branch','feature'),@('status','in_progress'),@('conclusion','failure'),@('conclusion','cancelled'),@('path','.github/workflows/ci.yml'),@('id','1;x'))) {
    $bad=$good.PSObject.Copy();$bad.($change[0])=$change[1]
    Assert-RejectedWith {Assert-InstallerE2ERun $bad $revision} 'green installer e2e run'
}
$bad=$good.PSObject.Copy();$bad.head_repository=[pscustomobject]@{full_name='fork/mmhmem'}
Assert-RejectedWith {Assert-InstallerE2ERun $bad $revision} 'green installer e2e run'
# The aggregate job decides, not only the run conclusion.
foreach ($jobs in @(@(),@((New-GateJob 'failure')),@((New-GateJob 'skipped')),@((New-GateJob),(New-GateJob)),@((New-GateJob 'success' 'e2e linux-x86_64')))) {
    $script:E2EJobs=$jobs
    Assert-RejectedWith {Assert-InstallerE2ERun $good $revision} 'installer-e2e job to pass'
}
$script:E2EJobs=@((New-GateJob))
# Lookup: only push runs on main for the exact commit; the newest one decides.
$script:ApiCalls.Clear()
if ((Get-InstallerE2ERun $revision).id -ne 789) {throw 'Green push run was not found'}
if ($script:ApiCalls[0] -cne "actions/workflows/installer-e2e.yml/runs?branch=main&event=push&head_sha=$revision&per_page=100") {throw "Unexpected installer e2e query: $($script:ApiCalls[0])"}
# A green workflow_dispatch run (where sabotage=wrong-hash lives) is never evidence,
# even if the API returned it despite event=push.
$script:E2ERuns=@((New-E2ERun 900 'workflow_dispatch'))
Assert-RejectedWith {Get-InstallerE2ERun $revision} 'none finished in time'
$script:E2ERuns=@((New-E2ERun 700 'push' 'failure'),(New-E2ERun 900 'workflow_dispatch'))
Assert-RejectedWith {Assert-InstallerE2ERun (Get-InstallerE2ERun $revision) $revision} 'green installer e2e run'
# An older green run does not hide a newer red or unfinished one.
$script:E2ERuns=@((New-E2ERun 700),(New-E2ERun 701 'push' 'failure'))
Assert-RejectedWith {Assert-InstallerE2ERun (Get-InstallerE2ERun $revision) $revision} 'green installer e2e run'
$script:E2ERuns=@((New-E2ERun 700),(New-E2ERun 701 'push' '' 'in_progress'))
Assert-RejectedWith {Get-InstallerE2ERun $revision} 'none finished in time'
$script:E2ERuns=@()
Assert-RejectedWith {Get-InstallerE2ERun $revision} 'none finished in time'
Assert-RejectedWith {Get-InstallerE2ERun 'main'} 'Invalid revision'
# Publishing refuses a commit without a green installer run: with green main CI
# the publisher stops before any artifact download, release creation or upload.
foreach ($runs in @(@(),@((New-E2ERun 900 'workflow_dispatch')),@((New-E2ERun 701 'push' 'failure')),@((New-E2ERun 701 'push' '' 'in_progress')))) {
    $script:E2ERuns=$runs;$script:GhCalls.Clear();$script:ApiCalls.Clear()
    Assert-RejectedWith {Invoke-CiRelease 'v0.3.0' '123'} 'installer e2e run'
    if ($script:GhCalls.Count -ne 0) {throw "gh was called without a green installer run: $($script:GhCalls -join '; ')"}
    if (@($script:ApiCalls | Where-Object {$_ -like 'releases*'}).Count -ne 0) {throw 'Releases were read without a green installer run'}
}
$script:E2ERuns=@((New-E2ERun 789));$script:E2EJobs=@((New-GateJob 'failure'));$script:GhCalls.Clear()
Assert-RejectedWith {Invoke-CiRelease 'v0.3.0' '123'} 'installer-e2e job to pass'
if ($script:GhCalls.Count -ne 0) {throw 'gh was called with a red installer-e2e job'}
# With both gates green the publisher goes on to the artifact download.
$script:E2EJobs=@((New-GateJob));$script:GhCalls.Clear()
Assert-RejectedWith {Invoke-CiRelease 'v0.3.0' '123'} 'MOCK: reached the artifact download'
# The main CI gate still applies on its own.
$script:CiRun=[pscustomobject]$validRun;$script:CiRun.conclusion='failure';$script:GhCalls.Clear()
Assert-RejectedWith {Invoke-CiRelease 'v0.3.0' '123'} 'successful trusted main CI'
if ($script:GhCalls.Count -ne 0) {throw 'gh was called with red main CI'}
$script:CiRun=[pscustomobject]$validRun
# Draft until attestation: the build step never publishes, -PublishDraft does,
# after re-checking the tag, both gates and the exact draft asset set.
$publisherText=[IO.File]::ReadAllText((Join-Path $PSScriptRoot 'publish-ci-release.ps1'))
$buildBody=$publisherText.Substring($publisherText.IndexOf('function Invoke-CiRelease'))
$buildBody=$buildBody.Substring(0,$buildBody.IndexOf('function Publish-CiReleaseDraft'))
if ($buildBody.Contains('--draft=false')) {throw 'The build step must leave the release a draft'}
$verifiedNames='SHA256SUMS,SHA256SUMS.sig,mmhmem.exe'
function Set-Draft([string[]]$Names,[string]$State='uploaded') {$script:MockDraftAssets=@($Names | ForEach-Object {[pscustomobject]@{name=$_;state=$State}})}
function Get-EditCount {return @($script:GhCalls | Where-Object {$_ -ceq 'release edit v0.3.0 --repo norandom/mmhmem --draft=false --latest'}).Count}
Set-Draft @('mmhmem.exe','SHA256SUMS.sig','SHA256SUMS');$script:GhCalls.Clear()
Publish-CiReleaseDraft 'v0.3.0' '123' $revision $verifiedNames
if ((Get-EditCount) -ne 1 -or $script:GhCalls.Count -ne 1) {throw "Draft was not published exactly once: $($script:GhCalls -join '; ')"}
$refusals=@(
    @{Why='extra asset';Setup={Set-Draft @('mmhmem.exe','SHA256SUMS.sig','SHA256SUMS','unverified.bin')};Expect='Draft assets differ'},
    @{Why='missing asset';Setup={Set-Draft @('mmhmem.exe','SHA256SUMS')};Expect='Draft assets differ'},
    @{Why='incomplete asset';Setup={Set-Draft @('mmhmem.exe','SHA256SUMS.sig','SHA256SUMS') 'new'};Expect='Incomplete release asset'},
    @{Why='already published';Setup={$script:DraftState=$false};Expect='no longer a draft'},
    @{Why='tag moved';Setup={$script:TagRevision='c'*40};Expect='Tag changed'},
    @{Why='installer run turned red';Setup={$script:E2ERuns=@((New-E2ERun 789),(New-E2ERun 790 'push' 'failure'))};Expect='green installer e2e run'},
    @{Why='installer run re-running';Setup={$script:E2ERuns=@((New-E2ERun 789 'push' '' 'in_progress'))};Expect='none finished in time'},
    @{Why='CI turned red';Setup={$script:CiRun=[pscustomobject]$validRun;$script:CiRun.conclusion='failure'};Expect='successful trusted main CI'}
)
foreach ($case in $refusals) {
    Set-Draft @('mmhmem.exe','SHA256SUMS.sig','SHA256SUMS');$script:DraftState=$true;$script:TagRevision=$revision
    $script:E2ERuns=@((New-E2ERun 789));$script:E2EJobs=@((New-GateJob));$script:CiRun=[pscustomobject]$validRun;$script:GhCalls.Clear()
    & $case.Setup
    Assert-RejectedWith {Publish-CiReleaseDraft 'v0.3.0' '123' $revision $verifiedNames} $case.Expect
    if ((Get-EditCount) -ne 0) {throw "Draft was published despite: $($case.Why)"}
}
Assert-RejectedWith {Publish-CiReleaseDraft 'v0.3.0' '123' $revision ''} 'No verified asset list'
Assert-RejectedWith {Publish-CiReleaseDraft 'v0.3.0' '123' 'main' $verifiedNames} 'Invalid revision'
Assert-RejectedWith {Publish-CiReleaseDraft 'v0.3.0' 'x' $revision $verifiedNames} 'Invalid run ID'
# Workflow order: build the draft (only step with the key) -> attest -> publish.
$workflow=[IO.File]::ReadAllText((Join-Path $PSScriptRoot '..\.github\workflows\release.yml'))
$atDraft=$workflow.IndexOf('id: draft');$atAttest=$workflow.IndexOf('uses: actions/attest-build-provenance@');$atPublish=$workflow.IndexOf('./scripts/publish-ci-release.ps1 -PublishDraft')
if ($atDraft -lt 0 -or $atAttest -le $atDraft -or $atPublish -le $atAttest) {throw 'release.yml must build the draft, attest, then publish'}
if ($workflow.IndexOf('MMHMEM_RELEASE_SIGNING_KEY',$atAttest) -ge 0) {throw 'Only the draft step may see the signing key'}
if ($workflow.Substring($atAttest).Contains('continue-on-error')) {throw 'Attestation and publication must not continue on error'}
# Release notes (task 9.1): the draft is created with the template first, then GitHub's
# generated notes; the template opens with the two release one-liners.
$create=Get-CiReleaseCreateArguments 'v0.3.0'
$notesAt=[Array]::IndexOf($create,'--notes-file')
if (($create[0..2] -join ' ') -cne 'release create v0.3.0' -or $notesAt -lt 0 -or $create -cnotcontains '--generate-notes' -or $create -cnotcontains '--draft') {throw "Unexpected release create arguments: $($create -join ' ')"}
if ((Resolve-Path -LiteralPath $create[$notesAt+1]).Path -cne (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\.github\release-notes.md')).Path) {throw 'Release notes are not taken from .github/release-notes.md'}
if (-not $buildBody.Contains('Invoke-ReleaseGh (Get-CiReleaseCreateArguments $ReleaseTag)')) {throw 'Invoke-CiRelease does not create the draft with the release notes template'}
$notes=@([IO.File]::ReadAllLines($create[$notesAt+1]) | Where-Object {$_ -cmatch '^(irm|curl) '})
if ($notes.Count -ne 2 -or $notes[0] -cne 'irm https://github.com/norandom/mmhmem/releases/latest/download/install.ps1 | iex' -or
    $notes[1] -cne 'curl -fsSL https://github.com/norandom/mmhmem/releases/latest/download/install.sh | sh') {throw "Release notes do not start with the two one-liners: $($notes -join ' / ')"}
if (-not [IO.File]::ReadAllText($create[$notesAt+1]).Contains('gh attestation verify <archive> --repo norandom/mmhmem')) {throw 'Release notes lack the gh attestation verify line'}
Write-Host 'PASS: tag/run trust, exact native layout, identical retry, conflicts, published-release protection, staged assets, SHA256SUMS, no e2e-key builds, installer e2e gate (push runs only, newest decides, aggregate job), draft until attestation, release notes template'
