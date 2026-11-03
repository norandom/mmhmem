# Offline regression checks: bootstrap downloads are replaced with local fixtures.
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$scratch = Join-Path $repo ('tmp/install-tests-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $scratch | Out-Null
$originalTemp = $env:TEMP
$originalTmp = $env:TMP
$originalLocal = $env:LOCALAPPDATA
$originalPath = $env:PATH
[IO.File]::WriteAllText((Join-Path $scratch 'codex.cmd'), '@exit /b 0')
$env:PATH = $scratch + ';' + $env:PATH
$env:TEMP = $scratch
$env:TMP = $scratch
$env:LOCALAPPDATA = Join-Path $scratch 'profile'
$fixture = @{ DownloadBinary = $false; Install = $false }
function Invoke-WebRequest {
    param([switch]$UseBasicParsing, [Parameter(Position=0)][string]$Uri, [string]$OutFile)
    if ($Uri -like '*/releases/latest/download/mmhmem.exe') {
        $fixture.DownloadBinary = $true
        if ($fixture.Install) { [IO.File]::WriteAllText($OutFile, 'release fixture'); return }
        throw 'Dry run attempted to download a binary'
    }
    $relative = ($Uri -split '/HEAD/')[1]
    if (-not $relative) { throw "Unexpected URL: $Uri" }
    if ($fixture.Install -and $relative -eq 'deploy/install_local.ps1') {
        [IO.File]::WriteAllText($OutFile, @'
param([switch]$Global, [switch]$NonInteractive, [string]$Binary, [string]$Agent, [string]$InstallSource)
. (Join-Path $PSScriptRoot 'install_binary.ps1')
Install-MmhmemBinary -Source $InstallSource -Destination $Binary
'@); return
    }
    Copy-Item -LiteralPath (Join-Path $repo $relative) -Destination $OutFile
}
try {
    if ([Console]::IsInputRedirected) {
        $refused = $false
        try { & (Join-Path $repo 'install.ps1') -DryRun } catch { $refused = $_ -match 'Agent selection needs a terminal' }
        if (-not $refused) { throw 'Unattended bootstrap silently selected agents' }
    }
    $output = & (Join-Path $repo 'install.ps1') -DryRun -Agent codex 6>&1 | Out-String
    if ($output -notmatch 'copy SKILL.md' -or $output -notmatch '\.agents[\\/]skills[\\/]mmhmem-recap[\\/]SKILL.md' -or $output -notmatch 'config.toml') {
        throw "Dry run missed registration, skill or recap: $output"
    }
    if ($fixture.DownloadBinary -or (Test-Path $env:LOCALAPPDATA)) { throw 'Dry run wrote installation state' }
    $output = & (Join-Path $repo 'install.ps1') -Remove -DryRun -Agent codex 6>&1 | Out-String
    if ($output -notmatch 'local mode removed' -or $output -notmatch 'unified-memory') { throw "Removal failed: $output" }
    # One-line installer passes the hook options through (scratch Claude config; nothing is written).
    $originalClaudeDir = $env:CLAUDE_CONFIG_DIR
    $env:CLAUDE_CONFIG_DIR = Join-Path $scratch 'claude-config'
    try {
        $fixture.DownloadBinary = $false
        $output = & (Join-Path $repo 'install.ps1') -RemoveHooks -DryRun 6>&1 | Out-String
        if ($output -notmatch 'no Claude Code read-routing hook' -or $fixture.DownloadBinary -or (Test-Path $env:CLAUDE_CONFIG_DIR)) { throw "-RemoveHooks bootstrap: $output" }
        $refused = $false
        try { & (Join-Path $repo 'install.ps1') -Hooks codex -Agent codex -DryRun 6>&1 | Out-Null } catch { $refused = $_ -match 'Installer failed' }
        if (-not $refused) { throw '-Hooks codex was not passed through to the engine' }
    } finally { $env:CLAUDE_CONFIG_DIR = $originalClaudeDir }
    # Actual bootstrap writes, isolated from the user's harness registrations.
    $fixture.Install = $true
    & (Join-Path $repo 'install.ps1') -Agent codex
    $installed = Join-Path $env:LOCALAPPDATA 'Programs/mmhmem/mmhmem.exe'
    [IO.File]::WriteAllText($installed, 'old version')
    & (Join-Path $repo 'install.ps1') -Agent codex
    if ([IO.File]::ReadAllText($installed) -ne 'release fixture') { throw 'Bootstrap did not replace existing binary' }
    if (@(Get-ChildItem $scratch -Directory -Filter 'mmhmem-install-*').Count -ne 0) { throw 'Bootstrap leaked staging files' }
    $tokens = $null; $errors = $null
    foreach ($file in @('install.ps1','deploy/install_local.ps1','deploy/install_binary.ps1','deploy/hooks_onboarding.ps1','lib.ps1','build-marketplace.ps1')) {
        [Management.Automation.Language.Parser]::ParseFile((Join-Path $repo $file), [ref]$tokens, [ref]$errors) | Out-Null
        if ($errors.Count) { throw "$file parse errors: $errors" }
    }
    Write-Host 'PASS: PowerShell syntax, checkout-free dry run, Codex skill/recap, removal without companions, staging cleanup'
    & (Join-Path $PSScriptRoot 'test-routing-onboarding.ps1')
    & (Join-Path $PSScriptRoot 'test-routing-instructions.ps1')
    & (Join-Path $PSScriptRoot 'test-routing-cross.ps1')
    & (Join-Path $PSScriptRoot 'test-codex-registration.ps1')
    & (Join-Path $PSScriptRoot 'test-recap-skill.ps1')
    & (Join-Path $PSScriptRoot 'test-install-binary.ps1')
    & (Join-Path $PSScriptRoot 'test-install-menus.ps1')
    & (Join-Path $PSScriptRoot 'test-worker-check.ps1')
    & (Join-Path $PSScriptRoot 'test-manage-hooks.ps1')
    & (Join-Path $PSScriptRoot 'test-hooks-onboarding.ps1')
} finally {
    $env:TEMP = $originalTemp
    $env:TMP = $originalTmp
    $env:LOCALAPPDATA = $originalLocal
    $env:PATH = $originalPath
}
