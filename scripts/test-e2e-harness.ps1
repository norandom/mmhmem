#Requires -Version 5.1
# Offline checks of the installer end-to-end harness logic (task 8.2): the
# home comparison rules, the parsed ~/.claude.json comparison, and the broken
# releases the failure scenarios use. Runs no installer and no agent CLI, and
# writes only into a temporary folder. Windows PowerShell 5.1 and PowerShell 7.
$ErrorActionPreference='Stop'
$script:Root=Split-Path -Parent $PSScriptRoot
$script:OnWindows=[Environment]::OSVersion.Platform -eq [PlatformID]::Win32NT
$script:OnMac=$false
. (Join-Path $PSScriptRoot 'e2e-seed-home.ps1')
. (Join-Path $PSScriptRoot 'render-bootstraps.ps1')
function Write-Utf8([string]$Path,[string]$Text) {[IO.File]::WriteAllText($Path,$Text,(New-Object Text.UTF8Encoding($false)))}
function Read-Json([string]$Path) {[IO.File]::ReadAllText($Path) | ConvertFrom-Json}
function Write-Json([string]$Path,$Value) {Write-Utf8 $Path (($Value | ConvertTo-Json -Depth 8)+"`n")}
# The registry is never read here: a fixed PATH value stands in for it.
$script:FakePath=[ordered]@{exists=$true;kind='ExpandString';value='C:\a;C:\b'}
function Get-UserPathSnapshot {return [ordered]@{exists=$script:FakePath.exists;kind=$script:FakePath.kind;value=$script:FakePath.value}}

$work=Join-Path ([IO.Path]::GetTempPath()) ('mmhmem-e2e-harness-test-'+[guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $work | Out-Null
$script:WorkDirectory=$work
$WorkDir=$work
$script:SeedHome=Join-Path $work 'home'
$script:Manifest=Join-Path $work 'seed-manifest.sha256'
$script:Site=Join-Path $work 'site'
$script:Logs=Join-Path $work 'logs'
New-Item -ItemType Directory -Path $script:Logs | Out-Null
. (Join-Path $PSScriptRoot 'e2e-home-state.ps1')
. (Join-Path $PSScriptRoot 'e2e-scenarios.ps1')

$script:Checks=0
function Assert-True([bool]$Condition,[string]$What) {
    if (-not $Condition) {throw "FAILED: $What"}
    $script:Checks++
}
function Assert-Failures($Failures,[int]$Count,[string]$What,[string]$Contains='') {
    $list=@($Failures | Where-Object {$_})
    Assert-True ($list.Count -eq $Count) "$What (got $($list.Count): $($list -join ' | '))"
    if ($Contains) {Assert-True (($list -join "`n").Contains($Contains)) "$What mentions '$Contains' ($($list -join ' | '))"}
}

try {
    & (Join-Path $PSScriptRoot 'e2e-seed-home.ps1') -Destination $script:SeedHome -Manifest $script:Manifest
    $seedText=[IO.File]::ReadAllText((Join-Path $script:SeedHome '.claude.json'))

    # ---- canonical JSON: key order and whitespace do not matter, values do.
    $a=ConvertTo-CanonicalJson ('{"b":[1,{"y":true,"x":null}],"a":"s"}' | ConvertFrom-Json)
    $b=ConvertTo-CanonicalJson ("{`n `"a`" : `"s`",`n `"b`": [1, {`"x`": null, `"y`": true}]}" | ConvertFrom-Json)
    Assert-True ($a -ceq $b) 'canonical JSON ignores key order and whitespace'
    Assert-True ($a -cne (ConvertTo-CanonicalJson ('{"a":"s","b":[{"x":null,"y":true},1]}' | ConvertFrom-Json))) 'array order matters'

    # ---- ~/.claude.json, parsed.
    $seed=$seedText | ConvertFrom-Json
    # As Claude Code rewrites it: its own formatting, counters, new keys, a project for the home.
    $rewritten=$seedText | ConvertFrom-Json
    $rewritten.numStartups=43
    $rewritten | Add-Member -NotePropertyName userID -NotePropertyValue 'abc123'
    $rewritten.projects | Add-Member -NotePropertyName '/home/runner/e2e' -NotePropertyValue ([pscustomobject]@{allowedTools=@();hasTrustDialogAccepted=$false})
    $claudeLike=$rewritten | ConvertTo-Json -Depth 10
    Assert-Failures (Compare-ClaudeJson $seedText $claudeLike 'unchanged') 0 'a Claude Code rewrite keeps every unrelated setting'
    Assert-Failures (Compare-ClaudeJson $seedText '{ not json' 'unchanged') 1 'broken JSON is a failure' 'no longer valid JSON'
    Assert-Failures (Compare-ClaudeJson $seedText $null 'unchanged') 1 'a missing file is a failure' 'missing'

    $withMmhmem=$seedText | ConvertFrom-Json
    $withMmhmem.mcpServers | Add-Member -NotePropertyName mmhmem -NotePropertyValue ([pscustomobject]@{type='stdio';command='/x/mmhmem';args=@('stdio')})
    $installed=$withMmhmem | ConvertTo-Json -Depth 10
    Assert-Failures (Compare-ClaudeJson $seedText $installed 'installed') 0 'installed: the mmhmem entry is expected'
    Assert-Failures (Compare-ClaudeJson $seedText $installed 'unchanged') 1 'unchanged: a leftover mmhmem entry fails' 'mcpServers.mmhmem was added'
    Assert-Failures (Compare-ClaudeJson $seedText $seedText 'installed') 1 'installed: no mmhmem entry fails' 'no mcpServers.mmhmem'

    $changedUser=$seedText | ConvertFrom-Json
    $changedUser.mcpServers.'user-notes'.command='python'
    Assert-Failures (Compare-ClaudeJson $seedText ($changedUser | ConvertTo-Json -Depth 10) 'unchanged') 1 "another server's entry is strict" 'mcpServers.user-notes changed'
    $lostServer=$seedText | ConvertFrom-Json
    $lostServer.mcpServers.PSObject.Properties.Remove('remote-docs')
    Assert-Failures (Compare-ClaudeJson $seedText ($lostServer | ConvertTo-Json -Depth 10) 'unchanged') 1 'a removed server fails' 'remote-docs was removed'
    $theme=$seedText | ConvertFrom-Json
    $theme.theme='light'
    Assert-Failures (Compare-ClaudeJson $seedText ($theme | ConvertTo-Json -Depth 10) 'unchanged') 1 'a seeded setting is strict' 'key theme changed'
    $project=$seedText | ConvertFrom-Json
    $project.projects.'/work/example'.hasTrustDialogAccepted=$false
    Assert-Failures (Compare-ClaudeJson $seedText ($project | ConvertTo-Json -Depth 10) 'unchanged') 1 'a seeded project is strict' 'projects[/work/example] changed'
    $sneaky=$seedText | ConvertFrom-Json
    $sneaky | Add-Member -NotePropertyName cachedServers -NotePropertyValue @('mmhmem')
    Assert-Failures (Compare-ClaudeJson $seedText ($sneaky | ConvertTo-Json -Depth 10) 'unchanged') 1 'a new key mentioning mmhmem fails' 'cachedServers'
    $lost=$seedText | ConvertFrom-Json
    $lost.PSObject.Properties.Remove('hasCompletedOnboarding')
    Assert-Failures (Compare-ClaudeJson $seedText ($lost | ConvertTo-Json -Depth 10) 'unchanged') 1 'a lost seeded key fails' 'lost the key hasCompletedOnboarding'

    # ---- home states.
    $seeded=Get-HomeState 'seeded'
    Assert-True ($seeded.files.Count -eq 12) 'every seeded file is in the state'
    Assert-Failures (Compare-HomeState $seeded (Get-HomeState 'same') -Strict) 0 'an untouched home is identical'
    $codexLog=Join-Path $script:SeedHome '.codex/log/codex-tui.log'
    New-Item -ItemType Directory -Path (Split-Path -Parent $codexLog) -Force | Out-Null
    Write-Utf8 $codexLog 'log'
    Assert-Failures (Compare-HomeState $seeded (Get-HomeState 'agent-log')) 0 "an agent CLI's own log is allowed"
    Assert-Failures (Compare-HomeState $seeded (Get-HomeState 'agent-log-strict') -Strict) 1 'strict allows nothing' 'new file: .codex/log/codex-tui.log'
    Remove-Item -LiteralPath (Split-Path -Parent $codexLog) -Recurse -Force
    if ($script:OnWindows) {
        $shellCache=Join-Path $script:SeedHome 'AppData/Local/Microsoft/Windows/PowerShell/StartupProfileData-NonInteractive'
        New-Item -ItemType Directory -Path (Split-Path -Parent $shellCache) -Force | Out-Null
        Write-Utf8 $shellCache 'cache'
        Assert-Failures (Compare-HomeState $seeded (Get-HomeState 'shell-cache') -Strict) 0 "the Windows shell's own cache is allowed even when strict"
        Remove-Item -LiteralPath (Join-Path $script:SeedHome 'AppData') -Recurse -Force
    }
    # A seeded file is strict even where an agent pattern would match.
    $script:AgentRuntimePatterns+='.codex/config.toml'
    [IO.File]::AppendAllText((Join-Path $script:SeedHome '.codex/config.toml'),"`n")
    Assert-Failures (Compare-HomeState $seeded (Get-HomeState 'seeded-changed')) 1 'a seeded file never falls under the agent rule' 'changed file: .codex/config.toml'
    Assert-Failures (Compare-HomeState $seeded (Get-HomeState 'seeded-expected') @('.codex/config.toml')) 0 'an expected change is allowed'
    $script:AgentRuntimePatterns=@($script:AgentRuntimePatterns | Where-Object {$_ -cne '.codex/config.toml'})
    $leftover=Join-Path $script:SeedHome '.claude/skills/unified-memory/SKILL.md'
    New-Item -ItemType Directory -Path (Split-Path -Parent $leftover) -Force | Out-Null
    Write-Utf8 $leftover 'x'
    Assert-Failures (Compare-HomeState $seeded (Get-HomeState 'leftover') @('.codex/config.toml')) 1 'a file mmhmem left behind fails' 'new file: .claude/skills/unified-memory/SKILL.md'
    Remove-Item -LiteralPath (Join-Path $script:SeedHome '.claude/skills') -Recurse -Force
    Write-Utf8 (Join-Path $script:SeedHome '.claude.json') $claudeLike
    $rewrittenState=Get-HomeState 'claude-rewrite'
    Assert-Failures (Compare-HomeState $seeded $rewrittenState @('.codex/config.toml')) 0 'a Claude Code rewrite of ~/.claude.json passes parsed'
    Assert-Failures (Compare-HomeState $seeded $rewrittenState @('.codex/config.toml') -Strict) 1 'strict compares ~/.claude.json bytes' 'changed file: .claude.json'
    Write-Utf8 (Join-Path $script:SeedHome '.claude.json') $installed
    Assert-Failures (Compare-HomeState $seeded (Get-HomeState 'claude-mmhmem') @('.codex/config.toml')) 1 'a leftover mmhmem entry in ~/.claude.json fails' 'mcpServers.mmhmem was added'
    Assert-Failures (Compare-HomeState $seeded (Get-HomeState 'claude-installed') @('.codex/config.toml') -ClaudeMode installed) 0 'installed mode accepts the entry'
    $back=Read-HomeState 'claude-installed'
    Assert-True ($back.files.Count -eq $seeded.files.Count -and $back.claude_json -ceq $installed) 'a saved state reads back'
    if ($script:OnWindows) {
        $script:FakePath.value='C:\a;C:\b;C:\mmhmem'
        Assert-Failures (Compare-HomeState $seeded (Get-HomeState 'path') @('.codex/config.toml','.claude.json')) 1 'a changed user PATH fails' 'HKCU\Environment\Path changed'
        Assert-Failures (Compare-HomeState $seeded (Get-HomeState 'path-ok') @('.codex/config.toml','.claude.json','HKCU:Path')) 0 'an expected PATH change is allowed'
        $script:FakePath.value='C:\a;C:\b'
    }

    # ---- broken releases for download-404 and bad-hash.
    $tag='v0.0.9'
    $tagDir=Join-Path $script:Site $tag
    New-Item -ItemType Directory -Path $tagDir -Force | Out-Null
    $i=0
    foreach ($t in (Get-ReleaseArchiveTargets $tag)) {$i++;[IO.File]::WriteAllBytes((Join-Path $tagDir $t.Archive),[byte[]](1..(10+$i)))}
    Write-Json (Join-Path $script:Site 'release.json') ([ordered]@{tag=$tag;folder=$tag;port=47801;base_url="http://127.0.0.1:47801/$tag"})
    $url=New-BrokenRelease 'e2e-missing-archives'
    Assert-True ($url -ceq 'http://127.0.0.1:47801/e2e-missing-archives') 'the 404 release has its own URL'
    $missing=@(Get-ChildItem -LiteralPath (Join-Path $script:Site 'e2e-missing-archives') | ForEach-Object {$_.Name} | Sort-Object)
    Assert-True (($missing -join ',') -ceq 'install.ps1,install.sh') "the 404 release has only the bootstraps ($($missing -join ','))"
    $sh=[IO.File]::ReadAllText((Join-Path $script:Site 'e2e-missing-archives/install.sh'))
    Assert-True ($sh.Contains("BASE_URL='$url'")) 'the 404 bootstrap downloads from its own folder'
    $null=New-BrokenRelease 'e2e-wrong-hash' -WithArchives -WrongHash
    foreach ($t in (Get-ReleaseArchiveTargets $tag)) {
        $right=(Get-FileHash -LiteralPath (Join-Path $tagDir $t.Archive) -Algorithm SHA256).Hash.ToLowerInvariant()
        Assert-True (Test-Path -LiteralPath (Join-Path $script:Site "e2e-wrong-hash/$($t.Archive)")) "the bad-hash release serves $($t.Archive)"
        foreach ($name in @('install.sh','install.ps1')) {
            $text=[IO.File]::ReadAllText((Join-Path $script:Site "e2e-wrong-hash/$name"))
            if ($t.Key -ceq 'windows' -and $name -ceq 'install.sh') {continue}
            if ($t.Key -cne 'windows' -and $name -ceq 'install.ps1') {continue}
            Assert-True (-not $text.Contains($right)) "$name of the bad-hash release does not carry the right SHA-256 of $($t.Archive)"
            Assert-True ($text -cmatch "'[0-9a-f]{64}'") "$name still carries a well-formed SHA-256"
        }
    }

    # ---- small helpers.
    Assert-True ((ConvertTo-TagPattern 'v0.4.1') -ceq 'v0\.4\.1') 'tag pattern escapes dots only'
    Assert-True (@('windows','linux','macos-aarch64','macos-x86_64') -ccontains (Get-PlatformKey)) 'platform key'
    $one=Get-OneLinerProgram 'http://127.0.0.1:1/v1.0.0'
    if ($script:OnWindows) {
        Assert-True ($one.Args[2] -ceq "irm 'http://127.0.0.1:1/v1.0.0/install.ps1' | iex; exit `$LASTEXITCODE") 'the Windows one-liner in the terminal'
    } else {
        Assert-True ($one.Program -ceq '/bin/sh' -and $one.Args[1] -ceq "curl -fsSL 'http://127.0.0.1:1/v1.0.0/install.sh' | sh") 'the Unix one-liner in the terminal'
    }
    Write-Host "PASS: e2e harness logic ($script:Checks checks)"
} finally {
    Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
}
