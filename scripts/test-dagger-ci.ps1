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
    Write-Host 'PASS: native provenance, freshness, export containment, arguments and hosted identity'
} finally {
    # Remove junction itself first; never recursively traverse its target.
    $junction = Join-Path $scratch 'redirect'
    $junctionItem = Get-Item -LiteralPath $junction -Force -ErrorAction SilentlyContinue
    if ($junctionItem) { $junctionItem.Delete() }
    $resolved = [System.IO.Path]::GetFullPath($scratch)
    $allowed = [System.IO.Path]::GetFullPath((Join-Path $repo 'tmp')) + [System.IO.Path]::DirectorySeparatorChar
    if ($resolved.StartsWith($allowed, [System.StringComparison]::OrdinalIgnoreCase)) { Remove-Item -LiteralPath $resolved -Recurse -Force }
}
