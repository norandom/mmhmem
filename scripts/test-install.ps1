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
# The root install.ps1 is a transition delegator: it resolves the latest release tag and
# runs that tag's own installer (scripts/fixtures/release-install.ps1 stands in for it
# here), rewritten to the tag. All network access is replaced by the functions below; the
# production script has no URL override, so nothing here depends on an environment variable.
$tag = 'v9.9.9'
$delegator = Join-Path $repo 'install.ps1'
$releaseScript = [IO.File]::ReadAllText((Join-Path $repo 'scripts/fixtures/release-install.ps1'))
$fixture = @{ DownloadBinary = $false; Install = $false; PluginTemplate = $false; Tag = $tag; ApiFail = $false; ScriptText = $releaseScript; Urls = (New-Object Collections.Generic.List[string]) }
function Invoke-RestMethod {
    param([switch]$UseBasicParsing, [string]$Uri)
    $fixture.Urls.Add($Uri)
    if ($Uri -ne 'https://api.github.com/repos/norandom/mmhmem/releases/latest') { throw "Unexpected API URL: $Uri" }
    if ($fixture.ApiFail) { throw 'API fixture failure' }
    [pscustomobject]@{ tag_name = $fixture.Tag }
}
# Runs the fixture release installer the way the delegator does (pinned to the tag) for
# options the v0.4.1 parameter set of the delegator does not carry (-Hooks, -RemoveHooks).
function Invoke-ReleaseInstaller([hashtable]$Parameters) {
    $text = $releaseScript.Replace('https://raw.githubusercontent.com/norandom/mmhmem/HEAD', "https://raw.githubusercontent.com/norandom/mmhmem/$tag")
    $text = $text.Replace('https://github.com/norandom/mmhmem/releases/latest/download', "https://github.com/norandom/mmhmem/releases/download/$tag")
    & ([scriptblock]::Create($text)) @Parameters
}
function Invoke-WebRequest {
    param([switch]$UseBasicParsing, [Parameter(Position=0)][string]$Uri, [string]$OutFile)
    $fixture.Urls.Add($Uri)
    if ($Uri -eq "https://raw.githubusercontent.com/norandom/mmhmem/$($fixture.Tag)/install.ps1") {
        return [pscustomobject]@{ RawContentStream = (New-Object IO.MemoryStream (,[Text.Encoding]::UTF8.GetBytes($fixture.ScriptText))) }
    }
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
    if ([Console]::IsInputRedirected) {
        $refused = $false
        try { & $delegator -DryRun } catch { $refused = $_ -match 'Agent selection needs a terminal' }
        if (-not $refused) { throw 'Unattended bootstrap silently selected agents' }
    }
    # Delegator behavior: tag resolution, pinning, refusals, pass-through, no writes on failure.
    $output = & $delegator -DryRun -Agent codex 6>&1 | Out-String
    if ($fixture.Urls[0] -ne 'https://api.github.com/repos/norandom/mmhmem/releases/latest') { throw 'Delegator did not resolve the tag first' }
    if ($fixture.Urls[1] -ne "https://raw.githubusercontent.com/norandom/mmhmem/$tag/install.ps1") { throw 'Delegator did not fetch the tag installer' }
    if (@($fixture.Urls | Where-Object { $_ -match '/HEAD(/|$)' -or $_ -like '*/releases/latest/*' }).Count) { throw 'Delegator left an unpinned URL' }
    if (-not ($fixture.Urls -contains "https://raw.githubusercontent.com/norandom/mmhmem/$tag/deploy/install_local.ps1")) { throw 'Helper files were not pinned to the tag' }
    $fixture.Urls.Clear()
    function Assert-Refused([string]$Pattern, [string]$What) {
        $failed = $null
        try { & $delegator -DryRun -Agent codex 6>&1 | Out-Null } catch { $failed = [string]$_ }
        if (-not $failed -or $failed -notmatch $Pattern -or $failed -notmatch 'Nothing was changed') { throw "$What was not refused plainly: $failed" }
        if ($fixture.DownloadBinary -or (Test-Path $env:LOCALAPPDATA)) { throw "$What wrote state" }
        if (@($fixture.Urls | Where-Object { $_ -like '*/deploy/*' }).Count) { throw "$What downloaded helper files" }
        $fixture.Urls.Clear()
    }
    $fixture.ApiFail = $true; Assert-Refused 'look up the latest' 'Failed tag lookup'; $fixture.ApiFail = $false
    foreach ($bad in @('v1.2', 'v1.2.3-rc1', 'latest', 'x1.2.3', '')) {
        $fixture.Tag = $bad; Assert-Refused 'not a stable version' "Tag '$bad'"
    }
    $fixture.Tag = $tag
    $fixture.ScriptText = $releaseScript + "`n`$extra = 'https://example.test/mmhmem/HEAD/x'`n"
    Assert-Refused 'moving HEAD' 'Leftover HEAD reference'
    $fixture.ScriptText = $releaseScript + "`n`$more = 'https://github.com/norandom/mmhmem/releases/latest/other'`n"
    Assert-Refused 'latest release' 'Leftover releases/latest reference'
    $fixture.ScriptText = "# mmhmem-transition-delegator`n" + $releaseScript
    Assert-Refused 'recurse' 'Recursive delegator'
    # Parameters reach the tag's installer: a probe installer echoes what it received.
    $fixture.ScriptText = @'
[CmdletBinding()]
param([string]$Agent,[string]$WorkerModel,[string]$WorkerUrl,[string]$WorkerApiKey,[switch]$Remove,[switch]$DryRun)
Write-Host ("PROBE agent=$Agent model=$WorkerModel url=$WorkerUrl key=$WorkerApiKey remove=$($Remove.IsPresent) dry=$($DryRun.IsPresent) raw=https://raw.githubusercontent.com/norandom/mmhmem/HEAD rel=https://github.com/norandom/mmhmem/releases/latest/download")
'@
    $output = & $delegator -Agent codex -WorkerModel m1 -WorkerUrl u1 -WorkerApiKey k1 -Remove -DryRun 6>&1 | Out-String
    $expected = "PROBE agent=codex model=m1 url=u1 key=k1 remove=True dry=True raw=https://raw.githubusercontent.com/norandom/mmhmem/$tag rel=https://github.com/norandom/mmhmem/releases/download/$tag"
    if (($output -replace '\s+', ' ').Trim() -ne $expected) { throw "Parameters or rewriting wrong: $output" }
    $output = & $delegator 6>&1 | Out-String
    if ($output -notmatch 'agent= model= url= key= remove=False dry=False') { throw "Defaults not preserved: $output" }
    $fixture.ScriptText = $releaseScript
    $fixture.Urls.Clear()
    if (((Get-Command $delegator).Parameters.Keys | Where-Object { $_ -in 'Agent','WorkerModel','WorkerUrl','WorkerApiKey','Remove','DryRun' }).Count -ne 6) { throw 'Delegator lost the v0.4.1 parameters' }
    $output = & $delegator -DryRun -Agent codex 6>&1 | Out-String
    if ($output -notmatch 'copy SKILL.md' -or $output -notmatch '\.agents[\\/]skills[\\/]mmhmem-recap[\\/]SKILL.md' -or $output -notmatch 'config.toml') {
        throw "Dry run missed registration, skill or recap: $output"
    }
    if (-not $fixture.PluginTemplate) { throw 'Bootstrap omitted the OpenCode plugin template' }
    if ($fixture.DownloadBinary -or (Test-Path $env:LOCALAPPDATA)) { throw 'Dry run wrote installation state' }
    $output = & $delegator -Remove -DryRun -Agent codex 6>&1 | Out-String
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
    & $delegator -Agent codex
    $installed = Join-Path $env:LOCALAPPDATA 'Programs/mmhmem/mmhmem.exe'
    [IO.File]::WriteAllText($installed, 'old version')
    & $delegator -Agent codex
    if ([IO.File]::ReadAllText($installed) -ne 'release fixture') { throw 'Bootstrap did not replace existing binary' }
    if (@(Get-ChildItem $scratch -Directory -Filter 'mmhmem-install-*').Count -ne 0) { throw 'Bootstrap leaked staging files' }
    $tokens = $null; $errors = $null
    foreach ($file in @('install.ps1','scripts/fixtures/release-install.ps1','deploy/install_local.ps1','deploy/install_binary.ps1','deploy/hooks_onboarding.ps1','lib.ps1','build-marketplace.ps1')) {
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
    & (Join-Path $PSScriptRoot 'test-manage-hooks-codex.ps1')
    & (Join-Path $PSScriptRoot 'test-hooks-onboarding.ps1')
} finally {
    $env:TEMP = $originalTemp
    $env:TMP = $originalTmp
    $env:LOCALAPPDATA = $originalLocal
    $env:PATH = $originalPath
}
