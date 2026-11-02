#Requires -Version 5.1
# Offline provenance tests: fake gh never contacts GitHub or writes releases.
$ErrorActionPreference = 'Stop'
$repositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$publisher = Join-Path $PSScriptRoot 'publish-linux-release.ps1'
if (-not (Test-Path -LiteralPath $publisher)) { throw 'Missing native Linux release publisher validator' }
. $publisher
$scratch = Join-Path $repositoryRoot ('tmp/publisher-tests-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $scratch -Force | Out-Null
$script:ReleaseScratchRoot=Join-Path $scratch 'downloads'
$script:Revision = 'a' * 40
$script:Mode = 'valid'
$script:Calls = New-Object System.Collections.Generic.List[string]
$script:RemoteAssets = @{}
$script:DeliveryWrites = New-Object System.Collections.Generic.List[string]
$script:FixtureRoot = Join-Path $scratch 'fixture'
$script:FakeArtifact = 'linux-static-' + $script:Revision + '-123-2'
$script:Cases = @('pristine_startup','version','mcp_initialize','tools_list','persistence','stats_text','stats_json','server_health','server_auth')
function Write-TestJson($Path, $Value) {
    [IO.File]::WriteAllText($Path, (($Value | ConvertTo-Json -Depth 40) + "`n"), [Text.UTF8Encoding]::new($false))
}
function New-TestBundle($Root) {
    New-Item -ItemType Directory -Path (Join-Path $Root 'reports') -Force | Out-Null
    [IO.File]::WriteAllText((Join-Path $Root 'mmhmem-linux-x86_64'), 'synthetic offline fixture; never execute', [Text.UTF8Encoding]::new($false))
    $artifact = [ordered]@{name='mmhmem-linux-x86_64';sha256=(Get-FileHash (Join-Path $Root 'mmhmem-linux-x86_64')).Hash.ToLowerInvariant();size_bytes=(Get-Item (Join-Path $Root 'mmhmem-linux-x86_64')).Length}
    $manifest = [ordered]@{schema_version=1;revision=$script:Revision;source_clean=$true;publishable=$true;validation_id='ci-123-2';package_version='0.2.2';target='x86_64-unknown-linux-musl';rustc_version='rustc 1.98.1';cargo_version='cargo 1.98.1';lockfile_sha256=('b'*64);build_image=('rust:1.98-slim-bookworm@sha256:'+('c'*64));native_tools=@{'musl-gcc'='fixture compiler';readelf='fixture reader'};artifact=$artifact;linkage=@{status='passed';report='reports/linkage.json'};regression=@{status='passed';report='reports/target-suite.json'};runtimes=@()}
    $manifest.scratch=@{status='passed';validation_id=$manifest.validation_id;artifact_sha256=$artifact.sha256;reported_version='mmhmem 0.2.2';report='reports/scratch.json'}
    Write-TestJson (Join-Path $Root 'reports/scratch.json') @{status='passed';validation_id=$manifest.validation_id;artifact_sha256=$artifact.sha256;reported_version='mmhmem 0.2.2'}
    $linkage = [ordered]@{status='passed';target=$manifest.target;artifact=$artifact;elf=@{class='ELF64';architecture='x86_64';type='DYN'};checks=@{};readelf_version='fixture readelf';readelf=@{header=" Class: ELF64`n Machine: Advanced Micro Devices X86-64`n Type: DYN (Position-Independent Executable file)";program=' LOAD 0x0000';dynamic='Dynamic section: static PIE metadata';versions='No version information found in this file.'}}
    foreach ($name in @('architecture','elf_class','interpreter','shared_libraries','glibc_versions')) { $linkage.checks[$name]=@{status='passed'} }
    Write-TestJson (Join-Path $Root 'reports/linkage.json') $linkage
    $regression = [ordered]@{status='passed';validation_id=$manifest.validation_id;reported_version='mmhmem 0.2.2';optional_telemetry=$false;active_dependencies=@('mmhmem','rustls','aws-lc-rs');checks=@{}}
    foreach ($key in @('target','artifact','package_version','lockfile_sha256','rustc_version','cargo_version','build_image','native_tools')) { $regression[$key]=$manifest[$key] }
    foreach ($name in @('target-dependencies','clippy','tests','release-build')) {
        $regression.checks[$name]=@{status='passed';report="reports/$name.txt"}
        [IO.File]::WriteAllText((Join-Path $Root "reports/$name.txt"), "synthetic successful diagnostics`n")
    }
    Write-TestJson (Join-Path $Root 'reports/target-suite.json') $regression
    foreach ($name in @('debian-bookworm','alpine-3.22')) {
        $image = if ($name -eq 'debian-bookworm') {'debian:bookworm-slim'} else {'alpine:3.22'}
        $runtime = [ordered]@{name=$name;image=($image+'@sha256:'+('d'*64));kernel='fixture-kernel';architecture='x86_64';cpu='fixture CPU';artifact_sha256=$artifact.sha256;report="reports/$name.json";cases=@{}}
        foreach ($case in $script:Cases) { $runtime.cases[$case]=@{status='passed'} }
        $runtime.cases.pristine_startup.artifact_sha256=$artifact.sha256
        $runtime.cases.pristine_startup.reported_version='mmhmem 0.2.2'
        $report = [ordered]@{status='passed';validation_id=$manifest.validation_id}
        foreach ($key in @('name','image','kernel','architecture','cpu','artifact_sha256','cases')) { $report[$key]=$runtime[$key] }
        Write-TestJson (Join-Path $Root $runtime.report) $report
        $manifest.runtimes += $runtime
    }
    Write-TestJson (Join-Path $Root 'verification.json') $manifest
    [IO.File]::WriteAllText((Join-Path $Root 'SHA256SUMS'), ($artifact.sha256 + "  mmhmem-linux-x86_64`n"), [Text.UTF8Encoding]::new($false))
}
function Change-TestDownload($Root) {
    $path = Join-Path $Root 'verification.json'
    $value = Get-Content -Raw -LiteralPath $path | ConvertFrom-Json
    switch ($script:Mode) {
        'dirty' {$value.source_clean=$false}
        'not-publishable' {$value.publishable=$false}
        'schema' {$value.schema_version=2}
        'bool-string' {$value.source_clean='true'}
        'hash' {$value.artifact.sha256='f'*64}
        'target' {$value.target='x86_64-unknown-linux-gnu'}
        'revision' {$value.revision='e'*40}
        'validation' {$value.validation_id='ci-123-1'}
        'runtime' {$value.runtimes[0].cases.server_auth.status='failed'}
        'duplicate-runtime' {$value.runtimes[1]=$value.runtimes[0]}
        'path' {$value.linkage.report='../outside.json'}
        'missing-field' {$value.PSObject.Properties.Remove('lockfile_sha256')}
        'bytes' {[IO.File]::AppendAllText((Join-Path $Root 'mmhmem-linux-x86_64'),'tamper')}
        'sums' {[IO.File]::WriteAllText((Join-Path $Root 'SHA256SUMS'),'wrong checksum')}
        'evidence' {
            $report = Get-Content -Raw (Join-Path $Root 'reports/alpine-3.22.json') | ConvertFrom-Json
            $report.cases.server_auth.status='failed'
            Write-TestJson (Join-Path $Root 'reports/alpine-3.22.json') $report
        }
        'dynamic' {
            $report = Get-Content -Raw (Join-Path $Root 'reports/linkage.json') | ConvertFrom-Json
            $report.readelf.program += "`n INTERP /lib/ld.so"
            Write-TestJson (Join-Path $Root 'reports/linkage.json') $report
        }
        'empty-log' {[IO.File]::WriteAllText((Join-Path $Root 'reports/tests.txt'),'')}
        'scratch' {
            $report=Get-Content -Raw (Join-Path $Root 'reports/scratch.json') | ConvertFrom-Json
            $report.status='failed'; Write-TestJson (Join-Path $Root 'reports/scratch.json') $report
        }
        'missing-scratch' {$value.PSObject.Properties.Remove('scratch')}
        'pristine' {$value.runtimes[0].cases.pristine_startup.reported_version='wrong version'}
    }
    Write-TestJson $path $value
    if ($script:Mode -eq 'duplicate-json') {
        $raw = Get-Content -Raw $path
        [IO.File]::WriteAllText($path, $raw.Replace('"schema_version":  1','"schema_version": 1, "schema_version": 1').Replace('"schema_version": 1,','"schema_version": 1, "schema_version": 1,'))
    }
}
function global:gh {
    $arguments = @($args)
    $script:Calls.Add(($arguments -join ' '))
    $global:LASTEXITCODE=0
    if ($arguments[0] -eq 'api') {
        if ($arguments.Count -ne 4 -or $arguments[1] -ne '--method' -or $arguments[2] -ne 'GET' -or $arguments[3] -notlike 'repos/norandom/mmhmem/*') {throw 'Unsafe metadata request'}
        $endpoint=$arguments[-1]
        if ($endpoint -like '*/git/ref/tags/*') {
            if ($script:Mode -eq 'missing-tag') { $global:LASTEXITCODE=1; return 'tag missing' }
            return (@{ref='refs/tags/v-test';object=@{type='tag';sha=('e'*40)}} | ConvertTo-Json -Depth 5 -Compress)
        }
        if ($endpoint -like '*/git/tags/*') {
            $resolved=if ($script:Mode -eq 'tag-moved') {'f'*40} else {$script:Revision}
            return (@{object=@{type='commit';sha=$resolved}} | ConvertTo-Json -Depth 5 -Compress)
        }
        if ($endpoint -like '*/actions/workflows/ci.yml') { return (@{id=22;path='.github/workflows/ci.yml';state='active'} | ConvertTo-Json -Compress) }
        if ($endpoint -like '*/actions/runs/123') {
            $run=@{id=123;run_attempt=2;name='CI';path='.github/workflows/ci.yml';workflow_id=22;status='completed';conclusion='success';event='push';head_branch='main';head_sha=$script:Revision;repository=@{full_name='norandom/mmhmem'};head_repository=@{full_name='norandom/mmhmem'}}
            switch ($script:Mode) {
                'wrong-repo' {$run.repository.full_name='other/repo'}
                'fork' {$run.head_repository.full_name='other/repo'}
                'wrong-workflow' {$run.path='.github/workflows/other.yml'}
                'wrong-workflow-id' {$run.workflow_id=99}
                'missing-workflow-id' {$run.Remove('workflow_id')}
                'string-workflow-id' {$run.workflow_id='22'}
                'failed-run' {$run.conclusion='failure'}
                'running' {$run.status='in_progress'}
                'wrong-commit' {$run.head_sha='f'*40}
                'untrusted-event' {$run.event='pull_request'}
                'string-attempt' {$run.run_attempt='2'}
            }
            return ($run | ConvertTo-Json -Depth 5 -Compress)
        }
        if ($endpoint -like '*/attempts/2/jobs*') {
            $jobs=@('Blocking quality gates (Dagger)','Windows x64 - clippy, deterministic tests, release build','macOS aarch64 and x86_64 - clippy, deterministic tests, release builds') | ForEach-Object {@{name=$_;status='completed';conclusion='success'}}
            if ($script:Mode -eq 'failed-job') {$jobs[1].conclusion='failure'}
            if ($script:Mode -eq 'skipped-job') {$jobs[1].conclusion='skipped'}
            if ($script:Mode -eq 'missing-job') {$jobs=@($jobs[0],$jobs[2])}
            if ($script:Mode -eq 'duplicate-job') {$jobs+=@($jobs[0])}
            return (@{total_count=$jobs.Count;jobs=@($jobs)} | ConvertTo-Json -Depth 6 -Compress)
        }
        if ($endpoint -like '*/artifacts*') {
            $items=@(@{name=$script:FakeArtifact;expired=$false})
            if ($script:Mode -eq 'missing-artifact') {$items=@()}
            if ($script:Mode -eq 'expired-artifact') {$items[0].expired=$true}
            if ($script:Mode -eq 'duplicate-artifact') {$items+=@($items[0])}
            return (@{total_count=$items.Count;artifacts=$items} | ConvertTo-Json -Depth 5 -Compress)
        }
        if ($endpoint -like '*/releases/tags/*') {
            if ($script:Mode -eq 'missing-release') {$global:LASTEXITCODE=1;return 'release missing'}
            $release=@{id=456;tag_name='v-test';draft=$true;assets=@(@{name='mmhmem.exe';id=77;state='uploaded'})}
            if ($script:Mode -eq 'published') {$release.draft=$false}
            if ($script:Mode -eq 'published-before-write') {$release.draft=$false}
            if ($script:FinalPublished) {$release.draft=$false}
            if ($script:Mode -eq 'published-mid-delivery' -and $script:RemoteAssets.Count) {$release.draft=$false}
            if ($script:Mode -eq 'release-changed') {$release.id=999}
            if ($script:Mode -in @('conflict','identical')) {$release.assets+=@{name='mmhmem-linux-x86_64';id=88;state='uploaded'}}
            foreach ($name in $script:RemoteAssets.Keys) {$release.assets+=@{name=$name;id=($script:RemoteAssets[$name].Id);state='uploaded'}}
            if ($script:Mode -eq 'remote-duplicate') {$release.assets+=@{name='mmhmem-linux-x86_64';id=998;state='uploaded'}}
            if ($script:Mode -eq 'remote-incomplete') {$release.assets[-1].state='starter'}
            if ($script:Mode -eq 'local-mid-delivery' -and $script:RemoteAssets.Count -and -not $script:ContextMutated) {
                [IO.File]::AppendAllText((Join-Path $script:CurrentContext.BundleDirectory 'verification.json'),'changed after first upload')
                $script:ContextMutated=$true
            }
            return ($release | ConvertTo-Json -Depth 5 -Compress)
        }
        throw "Unexpected fake API: $endpoint"
    }
    if ($arguments[0] -eq 'run' -and $arguments[1] -eq 'download') {
        if ($arguments[2] -ne '123' -or $arguments[[array]::IndexOf($arguments,'--name')+1] -ne $script:FakeArtifact) {throw 'Wrong artifact selection'}
        $target=$arguments[[array]::IndexOf($arguments,'--dir')+1]
        New-TestBundle $target
        Change-TestDownload $target
        return
    }
    if ($arguments[0] -eq 'release' -and $arguments[1] -eq 'download') {
        $target=$arguments[[array]::IndexOf($arguments,'--dir')+1]
        New-Item -ItemType Directory -Path $target -Force | Out-Null
        $name=$arguments[[array]::IndexOf($arguments,'--pattern')+1]
        if ($script:RemoteAssets.ContainsKey($name)) {
            $bytes=$script:RemoteAssets[$name].Bytes
            if ($script:Mode -eq 'corrupt-download') {$bytes=[Text.Encoding]::UTF8.GetBytes('corrupted remote response')}
            [IO.File]::WriteAllBytes((Join-Path $target $name),$bytes)
            if ($script:Mode -eq 'publish-on-last-download' -and $name -eq 'verification.json') {$script:FinalPublished=$true}
            return
        }
        $content=if ($script:Mode -eq 'identical') {'synthetic offline fixture; never execute'} else {'conflicting bytes'}
        [IO.File]::WriteAllText((Join-Path $target 'mmhmem-linux-x86_64'),$content,[Text.UTF8Encoding]::new($false))
        return
    }
    if ($arguments[0] -eq 'release' -and $arguments[1] -eq 'upload') {
        if ($arguments.Count -ne 6 -or $arguments[2] -ne 'v-test' -or $arguments[4] -ne '--repo' -or $arguments[5] -ne 'norandom/mmhmem') {throw 'Unsafe upload invocation'}
        $path=$arguments[3];$name=[IO.Path]::GetFileName($path)
        if ($name -notin @('mmhmem-linux-x86_64','SHA256SUMS','verification.json')) {throw 'Other platform upload attempted'}
        $script:DeliveryWrites.Add($name)
        if ($script:Mode -eq 'absent-ack') {$global:LASTEXITCODE=1;return 'network failure before upload'}
        if ($script:Mode -eq 'partial-failure' -and $name -eq 'SHA256SUMS') {$global:LASTEXITCODE=1;return 'interrupted second upload'}
        $bytes=if ($script:Mode -eq 'race-conflict') {[Text.Encoding]::UTF8.GetBytes('concurrent different asset')} else {[IO.File]::ReadAllBytes($path)}
        $script:RemoteAssets[$name]=@{Id=(100+$script:RemoteAssets.Count);Bytes=$bytes}
        if ($script:Mode -in @('lost-ack','race-conflict')) {$global:LASTEXITCODE=1;return 'lost upload acknowledgement'}
        return
    }
    throw "Forbidden or unexpected fake gh action: $($arguments -join ' ')"
}
try {
    foreach ($positive in @('valid','identical')) {
        $script:Mode=$positive; $script:Calls.Clear()
        $context=Invoke-LinuxReleasePublish -Tag 'v-test' -RunId '123' -ValidateOnly
        if ($context.Manifest.revision -ne $script:Revision -or $context.ReleaseId -ne 456) {throw 'Valid provenance was not retained'}
        if (@($script:Calls | Where-Object {$_ -match 'upload|create|edit|delete|clobber'}).Count) {throw 'Validation performed a write'}
    }
    $negative=@{
        'wrong-repo'='repository';'fork'='repository';'wrong-workflow'='workflow';'wrong-workflow-id'='workflow';'failed-run'='successful';'running'='successful';'wrong-commit'='commit';'untrusted-event'='trusted';'failed-job'='job';'skipped-job'='job';'missing-job'='job';'missing-artifact'='artifact';'missing-tag'='gh';'missing-release'='gh';'published'='draft';'conflict'='conflict';'dirty'='publishable';'not-publishable'='publishable';'schema'='schema';'bool-string'='boolean';'hash'='checksum';'target'='target';'revision'='revision';'validation'='validation';'runtime'='pass';'duplicate-runtime'='runtime';'path'='path';'missing-field'='lockfile';'bytes'='checksum';'sums'='SHA256SUMS';'evidence'='match|pass';'dynamic'='linkage';'empty-log'='empty';'duplicate-json'='duplicate'
    }
    $negative['missing-workflow-id']='workflow'; $negative['string-workflow-id']='workflow'; $negative['string-attempt']='trusted'
    $negative['duplicate-job']='job'; $negative['expired-artifact']='artifact'; $negative['duplicate-artifact']='artifact'
    $negative['scratch']='scratch|pass'; $negative['missing-scratch']='scratch'; $negative['pristine']='pristine|match'
    foreach ($mode in $negative.Keys) {
        $script:Mode=$mode; $script:Calls.Clear(); $caught=$false
        try {$null=Invoke-LinuxReleasePublish -Tag 'v-test' -RunId '123'} catch {
            $caught=$true
            if ($_.Exception.Message -notmatch $negative[$mode]) {throw "Wrong rejection for ${mode}: $($_.Exception.Message)"}
        }
        if (-not $caught) {throw "Accepted invalid case: $mode"}
        if (@($script:Calls | Where-Object {$_ -match 'upload|create|edit|delete|clobber'}).Count) {throw "Wrote before rejecting $mode"}
    }
    $script:Calls.Clear()
    try {$null=Invoke-LinuxReleaseValidation -Tag '../bad' -RunId '123';throw 'Bad tag accepted'} catch {if ($_.Exception.Message -eq 'Bad tag accepted') {throw}}
    if ($script:Calls.Count) {throw 'Invalid arguments reached gh'}
    $script:Calls.Clear(); $caught=$false
    try {$null=Invoke-LinuxReleaseValidation -Tag 'v-test' -RunId '123' -Binary 'arbitrary-local-file'} catch {$caught=$true}
    if (-not $caught -or $script:Calls.Count) {throw 'Arbitrary binary parameter was not rejected before gh'}
    Write-Host "PASS: 2 valid and $($negative.Count) offline provenance/bundle rejection cases; zero release writes"
    if (-not (Get-Command Invoke-LinuxReleaseDelivery -ErrorAction SilentlyContinue)) {throw 'Missing exact-byte draft delivery'}
    function New-DeliveryContext {
        $script:Mode='valid';$script:Calls.Clear();$script:RemoteAssets=@{};$script:DeliveryWrites.Clear();$script:ContextMutated=$false;$script:FinalPublished=$false
        return (Invoke-LinuxReleaseValidation -Tag 'v-test' -RunId '123')
    }
    function Assert-Delivered($Context) {
        foreach ($name in @('mmhmem-linux-x86_64','SHA256SUMS','verification.json')) {
            if (-not $script:RemoteAssets.ContainsKey($name)) {throw "Missing delivered $name"}
            $hash=[BitConverter]::ToString(([Security.Cryptography.SHA256]::Create()).ComputeHash($script:RemoteAssets[$name].Bytes)).Replace('-','').ToLowerInvariant()
            if ($hash -cne $Context.Hashes[$name]) {throw "Wrong delivered bytes: $name"}
        }
        if (@($script:Calls | Where-Object {$_ -match 'release (create|edit|delete)|clobber'}).Count) {throw 'Forbidden release mutation'}
    }
    $context=New-DeliveryContext
    $null=Invoke-LinuxReleasePublish -Tag 'v-test' -RunId '123'
    Assert-Delivered $context
    if (($script:DeliveryWrites -join ',') -ne 'mmhmem-linux-x86_64,SHA256SUMS,verification.json') {throw 'Expected exact three delivery files'}
    $writes=$script:DeliveryWrites.Count
    $null=Invoke-LinuxReleaseDelivery $context
    Assert-Delivered $context
    if ($script:DeliveryWrites.Count -ne $writes) {throw 'Identical retry uploaded again'}
    foreach ($deliveryCase in @('lost-ack','partial-failure')) {
        $context=New-DeliveryContext;$script:Mode=$deliveryCase;$failed=$false
        try {$null=Invoke-LinuxReleaseDelivery $context} catch {$failed=$true}
        if ($deliveryCase -eq 'lost-ack' -and $failed) {throw 'Lost acknowledgement was not reconciled'}
        if ($deliveryCase -eq 'partial-failure') {
            if (-not $failed -or -not $script:RemoteAssets.ContainsKey('mmhmem-linux-x86_64')) {throw 'Interrupted upload did not preserve first asset'}
            $script:Mode='valid';$script:DeliveryWrites.Clear();$null=Invoke-LinuxReleaseDelivery $context
            if ($script:DeliveryWrites.Contains('mmhmem-linux-x86_64')) {throw 'Partial retry replaced an existing asset'}
        }
        Assert-Delivered $context
    }
    foreach ($deliveryCase in @('published-before-write','published-mid-delivery','publish-on-last-download','release-changed','tag-moved','race-conflict','absent-ack','corrupt-download','local-mutation','local-mid-delivery','remote-conflict','remote-duplicate','remote-incomplete')) {
        $context=New-DeliveryContext;$script:Mode=$deliveryCase;$failed=$false;$script:CurrentContext=$context
        if ($deliveryCase -eq 'local-mutation') {[IO.File]::AppendAllText((Join-Path $context.BundleDirectory 'verification.json'),'tamper')}
        if ($deliveryCase -in @('remote-conflict','remote-duplicate','remote-incomplete')) {
            $script:RemoteAssets['mmhmem-linux-x86_64']=@{Id=100;Bytes=[Text.Encoding]::UTF8.GetBytes('conflicting existing asset')}
        }
        try {$null=Invoke-LinuxReleaseDelivery $context} catch {$failed=$true}
        if (-not $failed) {throw "Accepted invalid delivery: $deliveryCase"}
        if ($deliveryCase -in @('published-before-write','release-changed','tag-moved','local-mutation','remote-conflict','remote-duplicate','remote-incomplete') -and $script:DeliveryWrites.Count) {throw "Uploaded before rejecting $deliveryCase"}
        if ($deliveryCase -in @('published-mid-delivery','local-mid-delivery') -and $script:DeliveryWrites.Count -ne 1) {throw "Continued writing after state change: $deliveryCase"}
        if (@($script:Calls | Where-Object {$_ -match 'release (create|edit|delete)|clobber'}).Count) {throw "Forbidden mutation for $deliveryCase"}
    }
    Write-Host 'PASS: exact delivery, identical and partial retries, lost acknowledgements, state races, tampering and download equality'
} finally {
    Remove-Item Function:\gh -ErrorAction SilentlyContinue
    $resolved=[IO.Path]::GetFullPath($scratch)
    if (-not $resolved.StartsWith((Join-Path $repositoryRoot 'tmp')+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)) {throw 'Unsafe test cleanup path'}
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
