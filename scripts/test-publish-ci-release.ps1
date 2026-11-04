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
Write-Host 'PASS: tag/run trust, exact native layout, identical retry, conflicts, published-release protection, staged assets, SHA256SUMS and no e2e-key builds'
