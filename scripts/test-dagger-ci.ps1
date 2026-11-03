#Requires -Version 5.1
# Offline native-host regression checks; never starts WSL or Dagger.
$ErrorActionPreference = 'Stop'
$wrapper = Join-Path $PSScriptRoot 'dagger-ci.ps1'
$ast = [System.Management.Automation.Language.Parser]::ParseFile($wrapper, [ref]$null, [ref]$null)
if (-not $ast.Find({ param($node) $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Invoke-MmhmemCi' }, $true)) {
    throw 'Missing testable Invoke-MmhmemCi canonical wrapper'
}
. $wrapper
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$scratch = Join-Path $repo ('tmp/dagger-wrapper-tests-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $scratch -Force | Out-Null
$script:calls = New-Object System.Collections.Generic.List[object]
$script:dirty = ''
$script:fakeDestination = ''
$script:launchFailure = $false
function git {
    $script:calls.Add(@('git') + $args)
    $global:LASTEXITCODE = 0
    if ($args -contains 'rev-parse') { return ('a' * 40) }
    if ($args -contains 'status') { return $script:dirty }
    throw 'Unexpected fake git command'
}
function wsl {
    $script:calls.Add(@('wsl') + $args)
    if ($args -contains 'wslpath') { $global:LASTEXITCODE = 0; return '/mnt/c/fake/checkout' }
    if ($script:launchFailure) { Write-Error 'Fake native transport launch failure' -ErrorAction Continue; return }
    $global:LASTEXITCODE = 0
    if ($script:fakeDestination) {
        New-Item -ItemType Directory -Path $script:fakeDestination -Force | Out-Null
        $binary = Join-Path $script:fakeDestination 'mmhmem-linux-x86_64'
        [System.IO.File]::WriteAllBytes($binary, [byte[]](1, 2, 3))
        $validation = @($args | Where-Object { $_ -like '--validation-id=*' })[0].Substring(16)
        @{ revision = ('a' * 40); validation_id = $validation; source_clean = $false; publishable = $false;
           artifact = @{ sha256 = (Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash.ToLowerInvariant() } } |
            ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $script:fakeDestination 'verification.json')
    }
}
function Assert-True($condition, $message) { if (-not $condition) { throw $message } }
function Assert-Rejected([scriptblock]$operation, $label) {
    $script:calls.Clear()
    $rejected = $false
    try { & $operation | Out-Null } catch { $rejected = $true }
    Assert-True $rejected "$label was accepted"
    Assert-True (-not @($script:calls | Where-Object { $_[0] -eq 'wsl' }).Count) "$label reached WSL before rejection"
}
try {
    $script:calls.Clear()
    Assert-True ((Invoke-MmhmemCi -Call static-release) -eq 0) 'Clean source invocation failed'
    $first = @($script:calls | Where-Object { $_[0] -eq 'wsl' -and $_ -contains 'bash' })[0]
    $firstText = $first -join '|'
    Assert-True ($firstText.Contains('--revision=' + ('a' * 40))) 'Revision was not captured'
    Assert-True $firstText.Contains('--source-clean=true') 'Clean worktree was not recorded'
    Assert-True ($first -contains ('--revision=' + ('a' * 40))) 'Revision and cleanliness were combined into one argument'
    Assert-True ($first -contains '--source-clean=true') 'Cleanliness was not passed as its own argument'
    $firstId = @($first | Where-Object { $_ -like '--validation-id=*' })[0]
    Assert-True ($script:calls[0][0] -eq 'git' -and $script:calls[1][0] -eq 'git') 'Provenance was captured after WSL access'
    $script:calls.Clear()
    $script:dirty = '?? scratch-input'
    Assert-True ((Invoke-MmhmemCi -Call static-release) -eq 0) 'Dirty source invocation failed'
    $second = @($script:calls | Where-Object { $_[0] -eq 'wsl' -and $_ -contains 'bash' })[0]
    Assert-True (($second -join '|').Contains('--source-clean=false')) 'Dirty source became publishable'
    Assert-True (@($second | Where-Object { $_ -like '--validation-id=*' })[0] -ne $firstId) 'Validation identity was reused'
    $script:launchFailure = $true
    Assert-True ((Invoke-MmhmemCi -Call fmt) -ne 0) 'Native launch failure reused a previous successful exit code'
    $script:launchFailure = $false
    Assert-Rejected { Invoke-MmhmemCi -Call 'all;touch-owned' } 'Command injection'
    Assert-Rejected { Invoke-MmhmemCi -Call static-release -ExtraArgs @('--revision=' + ('b' * 40)) } 'Revision override'
    Assert-Rejected { Invoke-MmhmemCi -Call all -ExtraArgs @('--source-clean=true') } 'Cleanliness override'
    Assert-Rejected { Invoke-MmhmemCi -Call all -ExtraArgs @('--validation-id=old') } 'Validation override'
    Assert-Rejected { Invoke-MmhmemCi -Call all -ExtraArgs @('--src=elsewhere') } 'Source override'
    Assert-Rejected { Invoke-MmhmemCi -Call all -ExtraArgs @('--flag=ok;touch-owned') } 'Extra argument injection'
    Assert-Rejected { Invoke-MmhmemCi -Call static-release -ExportPath '../escape' } 'Outside export'
    Assert-Rejected { Invoke-MmhmemCi -Call static-release -ExportPath 'tmp/../escape' } 'Traversal export'
    Assert-Rejected { Invoke-MmhmemCi -Call release -ExportPath 'tmp/wrong-contract' } 'Non-bundle export'
    $existing = Join-Path $scratch 'existing'
    New-Item -ItemType Directory -Path $existing | Out-Null
    Set-Content -LiteralPath (Join-Path $existing 'keep') -Value 'existing evidence'
    Assert-Rejected { Invoke-MmhmemCi -Call static-release -ExportPath $existing } 'Existing destination'
    $junction = Join-Path $scratch 'redirect'
    New-Item -ItemType Junction -Path $junction -Target (Join-Path $repo 'ci') | Out-Null
    Assert-Rejected { Invoke-MmhmemCi -Call static-release -ExportPath (Join-Path $junction 'escaped') } 'Reparse export'
    (Get-Item -LiteralPath $junction -Force).Delete()
    $missingTarget = Join-Path $scratch 'missing-target'
    New-Item -ItemType Directory -Path $missingTarget | Out-Null
    New-Item -ItemType Junction -Path $junction -Target $missingTarget | Out-Null
    Remove-Item -LiteralPath $missingTarget
    Assert-Rejected { Invoke-MmhmemCi -Call static-release -ExportPath (Join-Path $junction 'escaped') } 'Dangling reparse export'
    $script:calls.Clear()
    $script:fakeDestination = Join-Path $scratch 'verified bundle'
    Invoke-MmhmemCi -Call all -ExportPath $script:fakeDestination | Out-Null
    $command = @($script:calls | Where-Object { $_[0] -eq 'wsl' -and $_ -contains 'bash' })[0]
    Assert-True (-not ($command -contains '-c')) 'Wrapper interpolates a shell command'
    Assert-True (($command -join '|').Contains('dagger-ci.sh')) 'Canonical parameterized transport missing'
    $workflow = Get-Content -Raw (Join-Path $repo '.github/workflows/ci.yml')
    Assert-True $workflow.Contains('linux-static-${{ github.sha }}-${{ github.run_id }}-${{ github.run_attempt }}') 'Hosted artifact identity missing'
    Assert-True $workflow.Contains('validation_id=ci-${GITHUB_RUN_ID}-${GITHUB_RUN_ATTEMPT}') 'Hosted validation identity missing'
    Assert-True $workflow.Contains('git status --porcelain') 'Hosted checkout cleanliness not checked'
    Assert-True $workflow.Contains('static-release --src ..') 'Hosted export bypasses verified gate'
    function Get-WorkflowJob([string]$text, [string]$job) {
        $match = [regex]::Match($text, '(?ms)^  ' + [regex]::Escape($job) + ':\r?\n(.*?)(?=^  [A-Za-z0-9_-]+:\r?$|\z)')
        Assert-True $match.Success "Workflow job $job missing"
        return $match.Groups[1].Value
    }
    $nativeJobs = @{ windows = 'mmhmem-x86_64-pc-windows-msvc'; macos = 'mmhmem-macos' }
    foreach ($os in @('windows', 'macos')) {
        $job = Get-WorkflowJob $workflow $os
        Assert-True (-not $job.Contains('uses: actions/cache@v4')) "$os still uses the combined cache action"
        Assert-True $job.Contains('uses: actions/cache/restore@v4') "$os cache restore step missing"
        Assert-True $job.Contains('uses: actions/cache/save@v4') "$os cache save step missing"
        Assert-True $job.Contains('id: cargo-cache') "$os cache restore step has no id"
        Assert-True $job.Contains("key: cargo-$os-1.98.1-`${{ hashFiles('Cargo.lock') }}") "$os primary cache key wrong"
        Assert-True ([regex]::IsMatch($job, "restore-keys: \|?\s*cargo-$os-1\.98\.1-\r?\n")) "$os restore-key prefix wrong"
        Assert-True $job.Contains('key: ${{ steps.cargo-cache.outputs.cache-primary-key }}') "$os save step does not reuse the primary key"
        Assert-True $job.Contains("if: success() && github.event_name == 'push' && steps.cargo-cache.outputs.cache-hit != 'true'") "$os save is not push-only on a non-exact hit"
        foreach ($path in @('~/.cargo/registry', '~/.cargo/git', 'target')) {
            Assert-True ([regex]::Matches($job, '(?m)^\s+' + [regex]::Escape($path) + '\r?$').Count -eq 2) "$os cache path $path not on both restore and save"
        }
        $restoreAt = $job.IndexOf('actions/cache/restore@v4'); $saveAt = $job.IndexOf('actions/cache/save@v4')
        $clippyAt = $job.IndexOf('cargo clippy'); $uploadAt = $job.IndexOf('actions/upload-artifact@v4')
        Assert-True ($restoreAt -ge 0 -and $restoreAt -lt $clippyAt -and $uploadAt -lt $saveAt) "$os cache steps are not around the cargo steps"
        $summary = [regex]::Match($job, '(?ms)- name: Report cargo cache status\r?\n(.*?)(?=^      - |\z)')
        Assert-True $summary.Success "$os cache summary step missing"
        $summaryText = $summary.Groups[1].Value
        Assert-True $summaryText.Contains('if: always()') "$os cache summary does not always run"
        Assert-True $summaryText.Contains('CACHE_HIT: ${{ steps.cargo-cache.outputs.cache-hit }}') "$os cache summary lacks the hit output"
        Assert-True $summaryText.Contains('CACHE_MATCHED_KEY: ${{ steps.cargo-cache.outputs.cache-matched-key }}') "$os cache summary lacks the matched key"
        Assert-True ($summaryText.IndexOf('${{') -lt $summaryText.IndexOf('run: ')) "$os cache summary inlines an expression into its script"
        Assert-True ($summaryText.Contains("'exact'") -and $summaryText.Contains('fallback (') -and $summaryText.Contains("'miss'")) "$os cache summary outcomes missing"
        Assert-True $summaryText.Contains('GITHUB_STEP_SUMMARY') "$os cache summary not written to the job summary"
        Assert-True $job.Contains("name: $($nativeJobs[$os])`n") "$os artifact name changed"
        Assert-True (-not $job.Contains('secrets.')) "$os job reads secrets"
    }
    Assert-True ((Get-WorkflowJob $workflow 'windows').Contains("path: target/release/mmhmem.exe`n")) 'Windows artifact path changed'
    Assert-True ((Get-WorkflowJob $workflow 'macos').Contains("path: |`n            target/release/mmhmem`n            target/x86_64-apple-darwin/release/mmhmem`n")) 'macOS artifact paths changed'
    Assert-True ((Get-WorkflowJob $workflow 'windows').Contains('./scripts/test-install.ps1')) 'Windows installer regression checks removed'
    $gateJob = Get-WorkflowJob $workflow 'quality-gates'
    $gateSteps = @([regex]::Split($gateJob, '(?m)^      - ') | Select-Object -Skip 1)
    $gateStep = @($gateSteps | Where-Object { $_ -match '(?m)^\s+args: all ' })
    Assert-True ($gateStep.Count -eq 1) 'Gate step running all not found exactly once'
    $gateStep = $gateStep[0]
    Assert-True ([regex]::IsMatch($gateStep, '(?m)^        id: gate\r?$')) 'Gate step has no id: gate'
    $gateArgs = [regex]::Match($gateStep, '(?m)^\s+args: (all .*?)\r?$').Groups[1].Value
    Assert-True ($gateArgs -match '(^| )--concurrency=3( |$)') 'Gate does not pass --concurrency=3'
    $gateJobs = [regex]::Match($gateArgs, '(?:^| )--cargo-jobs=(\d+)(?: |$)')
    Assert-True ($gateJobs.Success -and $gateJobs.Groups[1].Value -eq '4') 'Gate does not pass --cargo-jobs=4'
    foreach ($kept in @('--src ..', '--revision=${{ steps.provenance.outputs.revision }}', '--validation-id=${{ steps.provenance.outputs.validation_id }}', '--source-clean=true')) {
        Assert-True $gateArgs.Contains($kept) "Gate lost $kept"
    }
    $exportStep = @($gateSteps | Where-Object { $_ -match '(?m)^\s+args: static-release .*export --path=\.\./tmp/linux-static' })
    Assert-True ($exportStep.Count -eq 1) 'Export step not found exactly once'
    $exportArgs = [regex]::Match($exportStep[0], '(?m)^\s+args: (static-release .*?)\r?$').Groups[1].Value
    $exportJobs = [regex]::Match($exportArgs, '(?:^| )--cargo-jobs=(\d+)(?: |$)')
    Assert-True $exportJobs.Success 'Export step does not pass --cargo-jobs'
    Assert-True ($exportJobs.Groups[1].Value -eq $gateJobs.Groups[1].Value) 'Export --cargo-jobs differs from the gate (bundle would be rebuilt)'
    Assert-True ($exportArgs.IndexOf('--cargo-jobs=') -lt $exportArgs.IndexOf(' export ')) 'Export --cargo-jobs is not a static-release option'
    $summaryStep = @($gateSteps | Where-Object { $_.Contains('ci/scripts/timing_summary.py') })
    Assert-True ($summaryStep.Count -eq 1) 'Timing summary step missing'
    $summaryStep = $summaryStep[0]
    Assert-True ([regex]::IsMatch($summaryStep, '(?m)^        if: always\(\)\r?$')) 'Timing summary does not always run'
    Assert-True ([regex]::IsMatch($summaryStep, '(?m)^        env:\r?\n          GATE_OUTPUT: \$\{\{ steps\.gate\.outputs\.output \}\}\r?$')) 'Timing summary lacks GATE_OUTPUT from steps.gate.outputs.output'
    $summaryRun = [regex]::Match($summaryStep, '(?ms)^        run: (.*)$').Groups[1].Value
    Assert-True ($summaryRun.Contains('python3 ci/scripts/timing_summary.py')) 'Timing summary does not run the script with python3'
    Assert-True (-not $summaryRun.Contains('${{')) 'Timing summary inlines an expression into its script'
    Assert-True ($gateJob.IndexOf('id: gate') -lt $gateJob.IndexOf('ci/scripts/timing_summary.py')) 'Timing summary runs before the gate'
    Assert-True $gateJob.Contains('name: linux-static-${{ github.sha }}-${{ github.run_id }}-${{ github.run_attempt }}') 'Gate job artifact identity changed'
    $script:calls.Clear()
    $script:fakeDestination = ''
    Assert-True ((Invoke-MmhmemCi -Call all -ExtraArgs @('--concurrency=2', '--cargo-jobs=4')) -eq 0) 'Wrapper rejected concurrency/cargo-jobs options'
    $tuned = @($script:calls | Where-Object { $_[0] -eq 'wsl' -and $_ -contains 'bash' })[0]
    Assert-True (($tuned -contains '--concurrency=2') -and ($tuned -contains '--cargo-jobs=4')) 'Wrapper dropped concurrency/cargo-jobs options'
    Write-Host 'PASS: native provenance, freshness, export containment, arguments, hosted identity, gate tuning, timing summary and native caches'
} finally {
    # Remove junction itself first; never recursively traverse its target.
    $junction = Join-Path $scratch 'redirect'
    $junctionItem = Get-Item -LiteralPath $junction -Force -ErrorAction SilentlyContinue
    if ($junctionItem) { $junctionItem.Delete() }
    $resolved = [System.IO.Path]::GetFullPath($scratch)
    $allowed = [System.IO.Path]::GetFullPath((Join-Path $repo 'tmp')) + [System.IO.Path]::DirectorySeparatorChar
    if ($resolved.StartsWith($allowed, [System.StringComparison]::OrdinalIgnoreCase)) { Remove-Item -LiteralPath $resolved -Recurse -Force }
}
