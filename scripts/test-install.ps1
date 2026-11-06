# Offline regression checks: bootstrap downloads are replaced with local fixtures.
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$scratch = Join-Path $repo ('tmp/install-tests-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $scratch | Out-Null
$originalTemp = $env:TEMP
$originalTmp = $env:TMP
$originalLocal = $env:LOCALAPPDATA
$originalPath = $env:PATH
# Agent CLI stand-ins: the engine detects an agent by its command, so these checks must not
# depend on which agents this machine has installed (CI runners have none).
foreach ($cli in @('codex', 'opencode')) { [IO.File]::WriteAllText((Join-Path $scratch "$cli.cmd"), '@exit /b 0') }
$env:PATH = $scratch + ';' + $env:PATH
$env:TEMP = $scratch
$env:TMP = $scratch
$env:LOCALAPPDATA = Join-Path $scratch 'profile'
# The root install.ps1 is a stub (reliable-installers task 9.2): it prints the new release
# one-liner, downloads nothing, changes nothing and reports exit code 2. The old release
# installer keeps its coverage: scripts/fixtures/release-install.ps1 runs pinned to a tag,
# with all network access replaced by the functions below.
$tag = 'v9.9.9'
$stub = Join-Path $repo 'install.ps1'
$releaseScript = [IO.File]::ReadAllText((Join-Path $repo 'scripts/fixtures/release-install.ps1'))
$fixture = @{ DownloadBinary = $false; Install = $false; PluginTemplate = $false; Tag = $tag; Urls = (New-Object Collections.Generic.List[string]) }
function Invoke-RestMethod {
    param([switch]$UseBasicParsing, [Parameter(Position=0)][string]$Uri)
    $fixture.Urls.Add($Uri)
    throw "Unexpected API URL: $Uri"
}
# Runs the fixture release installer pinned to the tag, as the old transition delegator did.
function Invoke-ReleaseInstaller([hashtable]$Parameters) {
    $text = $releaseScript.Replace('https://raw.githubusercontent.com/norandom/mmhmem/HEAD', "https://raw.githubusercontent.com/norandom/mmhmem/$tag")
    $text = $text.Replace('https://github.com/norandom/mmhmem/releases/latest/download', "https://github.com/norandom/mmhmem/releases/download/$tag")
    & ([scriptblock]::Create($text)) @Parameters
}
function Invoke-WebRequest {
    param([switch]$UseBasicParsing, [Parameter(Position=0)][string]$Uri, [string]$OutFile)
    $fixture.Urls.Add($Uri)
    if ($Uri -like '*/releases/latest/*' -or $Uri -match '/HEAD(/|$)') { throw "Unpinned URL requested: $Uri" }
    if ($Uri -eq "https://github.com/norandom/mmhmem/releases/download/$($fixture.Tag)/mmhmem.exe") {
        $fixture.DownloadBinary = $true
        if ($fixture.Install) { [IO.File]::WriteAllText($OutFile, 'release fixture'); return }
        throw 'Dry run attempted to download a binary'
    }
    $relative = ($Uri -split "/mmhmem/$($fixture.Tag)/")[1]
    if ($relative -eq 'deploy/opencode/mmhmem-read-routing.js') { $fixture.PluginTemplate = $true }
    if (-not $relative) { throw "Unexpected URL: $Uri" }
    if ($fixture.ContainsKey('HookHome') -and $relative -eq 'deploy/install_local.ps1') {
        $source = [IO.File]::ReadAllText((Join-Path $repo $relative)).Replace('$HOME', '$BootstrapHookHome')
        $source = $source.Replace('$ErrorActionPreference = ''Stop''', ('$ErrorActionPreference = ''Stop''' + "`n" + '$BootstrapHookHome = ''' + $fixture.HookHome.Replace("'", "''") + ''''))
        [IO.File]::WriteAllText($OutFile, $source, (New-Object Text.UTF8Encoding $false))
        return
    }
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
    & (Join-Path $PSScriptRoot 'test-install-stub.ps1') -Stub $stub -Scratch (Join-Path $scratch 'stub')
    if ([Console]::IsInputRedirected) {
        $refused = $false
        try { Invoke-ReleaseInstaller @{ DryRun = $true } } catch { $refused = $_ -match 'Agent selection needs a terminal' }
        if (-not $refused) { throw 'Unattended bootstrap silently selected agents' }
    }
    # The release installer pins every helper download to the tag.
    $output = Invoke-ReleaseInstaller @{ DryRun = $true; Agent = 'codex' } 6>&1 | Out-String
    if (@($fixture.Urls | Where-Object { $_ -match '/HEAD(/|$)' -or $_ -like '*/releases/latest/*' }).Count) { throw 'Release installer left an unpinned URL' }
    if (-not ($fixture.Urls -contains "https://raw.githubusercontent.com/norandom/mmhmem/$tag/deploy/install_local.ps1")) { throw 'Helper files were not pinned to the tag' }
    $fixture.Urls.Clear()
    $output = Invoke-ReleaseInstaller @{ DryRun = $true; Agent = 'codex' } 6>&1 | Out-String
    if ($output -notmatch 'copy SKILL.md' -or $output -notmatch '\.agents[\\/]skills[\\/]mmhmem-recap[\\/]SKILL.md' -or $output -notmatch 'config.toml') {
        throw "Dry run missed registration, skill or recap: $output"
    }
    if (-not $fixture.PluginTemplate) { throw 'Bootstrap omitted the OpenCode plugin template' }
    if ($fixture.DownloadBinary -or (Test-Path $env:LOCALAPPDATA)) { throw 'Dry run wrote installation state' }
    $output = Invoke-ReleaseInstaller @{ Remove = $true; DryRun = $true; Agent = 'codex' } 6>&1 | Out-String
    if ($output -notmatch 'local mode removed' -or $output -notmatch 'unified-memory') { throw "Removal failed: $output" }
    # The release installer passes the hook options through (scratch Claude config; nothing is written).
    $originalClaudeDir = $env:CLAUDE_CONFIG_DIR
    $originalCodexDir = $env:CODEX_HOME
    $originalXdg = $env:XDG_CONFIG_HOME
    $env:XDG_CONFIG_HOME = $null
    $env:CLAUDE_CONFIG_DIR = Join-Path $scratch 'claude-config'
    $env:CODEX_HOME = Join-Path $scratch 'codex-config'
    try {
        $fixture.DownloadBinary = $false
        $output = Invoke-ReleaseInstaller @{ RemoveHooks = $true; DryRun = $true } 6>&1 | Out-String
        if ($output -notmatch 'no Claude Code read-routing hook' -or $fixture.DownloadBinary -or (Test-Path $env:CLAUDE_CONFIG_DIR)) { throw "-RemoveHooks bootstrap: $output" }
        $output = Invoke-ReleaseInstaller @{ Hooks = 'codex'; Agent = 'codex'; DryRun = $true } 6>&1 | Out-String
        if ($output -notmatch 'hook codex' -or $output -notmatch '/hooks') { throw '-Hooks codex was not passed through to the engine' }
        $fixture.HookHome = Join-Path $scratch 'hook-home'
        $env:XDG_CONFIG_HOME = Join-Path $fixture.HookHome '.config'
        $output = Invoke-ReleaseInstaller @{ Hooks = 'opencode'; Agent = 'opencode'; DryRun = $true } 6>&1 | Out-String
        if ($output -notmatch 'mmhmem-read-routing' -or $output -notmatch 'Sees read/shell') { throw 'OpenCode bootstrap opt-in missed staged template' }
    } finally { $fixture.Remove('HookHome'); $env:CLAUDE_CONFIG_DIR = $originalClaudeDir; $env:CODEX_HOME = $originalCodexDir; $env:XDG_CONFIG_HOME = $originalXdg }
    # Actual bootstrap writes, isolated from the user's harness registrations.
    $fixture.Install = $true
    Invoke-ReleaseInstaller @{ Agent = 'codex' }
    $installed = Join-Path $env:LOCALAPPDATA 'Programs/mmhmem/mmhmem.exe'
    [IO.File]::WriteAllText($installed, 'old version')
    Invoke-ReleaseInstaller @{ Agent = 'codex' }
    if ([IO.File]::ReadAllText($installed) -ne 'release fixture') { throw 'Bootstrap did not replace existing binary' }
    if (@(Get-ChildItem $scratch -Directory -Filter 'mmhmem-install-*').Count -ne 0) { throw 'Bootstrap leaked staging files' }
    $tokens = $null; $errors = $null
    foreach ($file in @('install.ps1','scripts/fixtures/release-install.ps1','deploy/install_local.ps1','deploy/install_binary.ps1','deploy/hooks_onboarding.ps1','lib.ps1','build-marketplace.ps1')) {
        [Management.Automation.Language.Parser]::ParseFile((Join-Path $repo $file), [ref]$tokens, [ref]$errors) | Out-Null
        if ($errors.Count) { throw "$file parse errors: $errors" }
    }
    Write-Host 'PASS: root stub, PowerShell syntax, checkout-free dry run, Codex skill/recap, removal without companions, staging cleanup'
    & (Join-Path $PSScriptRoot 'test-routing-onboarding.ps1')
    & (Join-Path $PSScriptRoot 'test-routing-instructions.ps1')
    & (Join-Path $PSScriptRoot 'test-routing-cross.ps1')
    & (Join-Path $PSScriptRoot 'test-codex-registration.ps1')
    & (Join-Path $PSScriptRoot 'test-recap-skill.ps1')
    & (Join-Path $PSScriptRoot 'test-install-binary.ps1')
    & (Join-Path $PSScriptRoot 'test-install-menus.ps1')
    & (Join-Path $PSScriptRoot 'test-worker-check.ps1')
    & (Join-Path $PSScriptRoot 'test-manage-hooks.ps1')
    & (Join-Path $PSScriptRoot 'test-manage-hooks-codex.ps1')
    & (Join-Path $PSScriptRoot 'test-hooks-onboarding.ps1')
} finally {
    $env:TEMP = $originalTemp
    $env:TMP = $originalTmp
    $env:LOCALAPPDATA = $originalLocal
    $env:PATH = $originalPath
}
# Every failure above throws. Some checks run native commands that are meant to fail, and
# a runner's `shell: powershell` exits with the last $LASTEXITCODE, so report success
# explicitly.
exit 0
