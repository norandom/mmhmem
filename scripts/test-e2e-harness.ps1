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
. (Join-Path $PSScriptRoot 'e2e-published.ps1')
. (Join-Path $PSScriptRoot 'sign-release-assets.ps1')

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

    # ---- Claude Code's own moves into ~/.claude/settings.json (research.md
    # section 8): autoUpdates false -> env.DISABLE_AUTOUPDATER "1", theme copied.
    $seedSettings=[IO.File]::ReadAllText((Join-Path $script:SeedHome '.claude/settings.json'))
    $migrated=$seedSettings | ConvertFrom-Json
    $migrated.env | Add-Member -NotePropertyName DISABLE_AUTOUPDATER -NotePropertyValue '1'
    $migrated | Add-Member -NotePropertyName theme -NotePropertyValue 'dark-daltonized'
    $migratedText=$migrated | ConvertTo-Json -Depth 10
    $moved=$claudeLike | ConvertFrom-Json
    $moved.PSObject.Properties.Remove('autoUpdates')
    $movedText=$moved | ConvertTo-Json -Depth 10
    Assert-Failures (Compare-ClaudeJson $seedText $movedText 'unchanged' $migratedText) 0 'autoUpdates moved by Claude Code into settings.json passes'
    Assert-Failures (Compare-ClaudeJson $seedText $movedText 'unchanged' $seedSettings) 1 'autoUpdates gone but not in settings.json fails' 'lost the key autoUpdates'
    Assert-Failures (Compare-ClaudeJson $seedText $movedText 'unchanged') 1 'autoUpdates gone without a settings.json fails' 'lost the key autoUpdates'
    $autoTrue=$seedText | ConvertFrom-Json
    $autoTrue.autoUpdates=$true
    Assert-Failures (Compare-ClaudeJson ($autoTrue | ConvertTo-Json -Depth 10) $movedText 'unchanged' $migratedText) 1 'only autoUpdates false is a known move' 'lost the key autoUpdates'
    Assert-Failures (Compare-ClaudeSettings $seedSettings $migratedText $seedText) 0 "Claude Code's settings.json rewrite passes parsed"
    Assert-Failures (Compare-ClaudeSettings $seedSettings $seedSettings $seedText) 0 'an unchanged settings.json passes'
    Assert-Failures (Compare-ClaudeSettings $seedSettings $migratedText ($autoTrue | ConvertTo-Json -Depth 10)) 1 'the env entry needs autoUpdates false before' 'env.DISABLE_AUTOUPDATER was added'
    $extraEnv=$migratedText | ConvertFrom-Json
    $extraEnv.env | Add-Member -NotePropertyName MMHMEM_X -NotePropertyValue '1'
    Assert-Failures (Compare-ClaudeSettings $seedSettings ($extraEnv | ConvertTo-Json -Depth 10) $seedText) 1 'another env entry fails' 'env.MMHMEM_X was added'
    $userEnv=$migratedText | ConvertFrom-Json
    $userEnv.env.EDITOR_HINT='nano'
    Assert-Failures (Compare-ClaudeSettings $seedSettings ($userEnv | ConvertTo-Json -Depth 10) $seedText) 1 "the user's env entry is strict" 'env.EDITOR_HINT changed'
    $otherTheme=$migratedText | ConvertFrom-Json
    $otherTheme.theme='light'
    Assert-Failures (Compare-ClaudeSettings $seedSettings ($otherTheme | ConvertTo-Json -Depth 10) $seedText) 1 'a copied theme must have the same value' 'key theme was added'
    $lostHooks=$migratedText | ConvertFrom-Json
    $lostHooks.PSObject.Properties.Remove('hooks')
    Assert-Failures (Compare-ClaudeSettings $seedSettings ($lostHooks | ConvertTo-Json -Depth 10) $seedText) 1 'a lost seeded setting fails' 'lost the key hooks'
    $newKey=$migratedText | ConvertFrom-Json
    $newKey | Add-Member -NotePropertyName enabledPlugins -NotePropertyValue ([pscustomobject]@{x=$true})
    Assert-Failures (Compare-ClaudeSettings $seedSettings ($newKey | ConvertTo-Json -Depth 10) $seedText) 1 'any other new key fails' 'key enabledPlugins was added'

    # ---- a log a program left running still writes to (OpenCode's service).
    $held=New-Object IO.FileStream((Join-Path $script:Logs 'held.out'),[IO.FileMode]::Create,[IO.FileAccess]::Write,[IO.FileShare]::ReadWrite)
    try {
        $bytes=[Text.Encoding]::UTF8.GetBytes("still running`n");$held.Write($bytes,0,$bytes.Length);$held.Flush()
        $refused=$false
        try {$null=[IO.File]::ReadAllText((Join-Path $script:Logs 'held.out'))} catch {$refused=$true}
        # Only Windows enforces the sharing mode between processes.
        if ($script:OnWindows) {Assert-True $refused 'ReadAllText refuses a log another program holds for writing'}
        Assert-True ((Read-Log 'held').Contains('still running')) 'Read-Log reads a log another program holds for writing'
    } finally {$held.Dispose()}

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
    # Files the agent CLIs write when setup checks them (research.md section 8):
    # OpenCode's service password and Claude Code's log of the mmhmem server.
    $agentFiles=@('.config/opencode/service.json',
        '.cache/claude-cli-nodejs/-home-runner-e2e/mcp-logs-mmhmem/2026-11-05T08-28-48-598Z.jsonl',
        'Library/Caches/claude-cli-nodejs/-Users-runner-e2e/mcp-logs-mmhmem/2026-11-05T08-28-59-268Z.jsonl',
        'AppData/Local/claude-cli-nodejs/Cache/D--a-e2e-home/mcp-logs-mmhmem/2026-11-05T08-29-00-000Z.jsonl')
    foreach ($rel in $agentFiles) {
        $file=Join-Path $script:SeedHome $rel
        New-Item -ItemType Directory -Path (Split-Path -Parent $file) -Force | Out-Null
        Write-Utf8 $file '{}'
    }
    $agentState=Get-HomeState 'agent-files'
    Assert-Failures (Compare-HomeState $seeded $agentState @('.codex/config.toml')) 0 "the agents' own service password and MCP logs are allowed"
    Assert-Failures (Compare-HomeState $seeded $agentState @('.codex/config.toml') -Strict) $agentFiles.Count 'strict allows none of them' 'new file: .config/opencode/service.json'
    foreach ($rel in $agentFiles) {Remove-Item -LiteralPath (Join-Path $script:SeedHome $rel) -Force}
    foreach ($rel in @('.config/opencode/other.json','.cache/claude-cli-nodejs/x/mcp-logs-other/a.jsonl','.cache/claude-cli-nodejs/x/mcp-logs-mmhmem/a.txt')) {
        $file=Join-Path $script:SeedHome $rel
        New-Item -ItemType Directory -Path (Split-Path -Parent $file) -Force | Out-Null
        Write-Utf8 $file '{}'
        Assert-Failures (Compare-HomeState $seeded (Get-HomeState 'agent-near') @('.codex/config.toml')) 1 "only those exact files are the agents' ($rel)" "new file: $rel"
        Remove-Item -LiteralPath $file -Force
    }
    foreach ($dir in @('.cache','Library','AppData')) {Remove-Item -LiteralPath (Join-Path $script:SeedHome $dir) -Recurse -Force -ErrorAction SilentlyContinue}
    Assert-Failures (Compare-HomeState $seeded (Get-HomeState 'agent-files-gone') @('.codex/config.toml') -Strict) 0 'the agent files are cleaned up again'
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
    # Claude Code's first run: ~/.claude.json loses autoUpdates, which lands in
    # the seeded ~/.claude/settings.json.
    Write-Utf8 (Join-Path $script:SeedHome '.claude.json') $movedText
    Write-Utf8 (Join-Path $script:SeedHome '.claude/settings.json') $migratedText
    $movedState=Get-HomeState 'claude-moved'
    Assert-True ($movedState.claude_settings -ceq $migratedText -and (Read-HomeState 'claude-moved').claude_settings -ceq $migratedText) 'a state keeps the settings.json text'
    Assert-Failures (Compare-HomeState $seeded $movedState @('.codex/config.toml')) 0 "Claude Code's moves into settings.json pass parsed"
    Assert-Failures (Compare-HomeState $seeded $movedState @('.codex/config.toml') -Strict) 2 'strict compares both files by bytes' 'changed file: .claude/settings.json'
    Write-Utf8 (Join-Path $script:SeedHome '.claude/settings.json') ($extraEnv | ConvertTo-Json -Depth 10)
    Assert-Failures (Compare-HomeState $seeded (Get-HomeState 'claude-moved-extra') @('.codex/config.toml')) 1 'an env entry Claude does not move fails' 'env.MMHMEM_X was added'

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

    # ---- programs setup must not leave running.
    $rows=@(
        [pscustomobject]@{Id=1;Name='mmhmem.exe';CommandLine='"D:\a\_temp\mmhmem-e2e\home\AppData\Local\Programs\mmhmem\mmhmem.exe" stdio'},
        [pscustomobject]@{Id=2;Name='mmhmem';CommandLine='/home/runner/work/_temp/mmhmem-e2e/home/.local/bin/mmhmem stdio'},
        [pscustomobject]@{Id=3;Name='opencode.exe';CommandLine='D:\a\_temp\agent-clis\node_modules\@opencode\cli\bin\opencode.exe serve --service'},
        [pscustomobject]@{Id=4;Name='opencode';CommandLine='/opt/agent-clis/lib/node_modules/@opencode/cli/bin/opencode serve --service'},
        [pscustomobject]@{Id=5;Name='opencode';CommandLine='opencode mcp list'},
        [pscustomobject]@{Id=6;Name='opencode';CommandLine='opencode serve --port 4096'},
        [pscustomobject]@{Id=7;Name='pwsh';CommandLine='pwsh -command . /home/runner/work/mmhmem/mmhmem/scripts/e2e-run.ps1 -Phase remove'},
        [pscustomobject]@{Id=8;Name='mmhmem-e2e-pty';CommandLine='/tmp/pty-target/release/mmhmem-e2e-pty remove.pty.txt'},
        [pscustomobject]@{Id=9;Name='.mmhmem.abcdefgh.__selfdelete__.exe';CommandLine='.mmhmem.abcdefgh.__selfdelete__.exe 123 x'})
    $left=@(Select-LeftoverProcesses $rows)
    Assert-True (($left -join '|') -ceq ((1..4 | ForEach-Object {"pid ${_}: $($rows[$_-1].CommandLine)"}) -join '|')) "leftovers are mmhmem and OpenCode's service only ($($left -join ' | '))"
    Assert-True (@(Get-ProcessRows | Where-Object {$_.Id -eq $PID}).Count -eq 1) 'the process list has this shell'
    # Piped the way the phases use it: one item per leftover, nothing when none.
    $realRows=${function:Get-ProcessRows}
    try {
        function Get-ProcessRows {return @($rows | Where-Object {$_.Id -ge 5})}
        $piped=@(Wait-NoLeftoverProcesses -Seconds 0 | ForEach-Object {"left: $_"})
        Assert-True ($piped.Count -eq 0) "no leftovers pipe no items ($($piped -join ' | '))"
        function Get-ProcessRows {return $rows}
        $piped=@(Wait-NoLeftoverProcesses -Seconds 0 | ForEach-Object {"left: $_"})
        Assert-True ($piped.Count -eq 4) "each leftover pipes as its own item ($($piped.Count))"
        function Get-ProcessRows {return @($rows[1])}
        $left=Wait-NoLeftoverProcesses -Seconds 0
        Assert-True ($left.Count -eq 1) 'one leftover counts as one'
    } finally {${function:Get-ProcessRows}=$realRows}

    # ---- the memory store, compared by content.
    $python=$null
    foreach ($candidate in @(@('py','-3'),@('python3'),@('python'))) {
        if ($python -or -not (Get-Command $candidate[0] -ErrorAction SilentlyContinue)) {continue}
        $found=& $candidate[0] @($candidate | Select-Object -Skip 1) -I -c 'import sys;print(sys.executable)' 2>$null
        if ($LASTEXITCODE -eq 0 -and $found) {$python=([string]$found).Trim()}
    }
    if (-not $python) {throw 'No Python 3 for the store checks'}
    function Get-Python {return $python}
    $storeDir=Join-Path $work 'store'
    New-Item -ItemType Directory -Path $storeDir | Out-Null
    $db=Join-Path $storeDir 'memory.db'
    Write-Utf8 (Join-Path $storeDir 'audit.jsonl') "{}`n"
    # Rows written and the program ended without closing: they sit in -wal.
    & $python -I -c "import os,sqlite3,sys;c=sqlite3.connect(sys.argv[1]);c.execute('pragma journal_mode=wal');c.execute('pragma wal_autocheckpoint=0');c.execute('create table t(a)');c.execute('insert into t values (1)');c.commit();os._exit(0)" $db
    Assert-True (Test-Path -LiteralPath "$db-wal") 'the test store has its rows in -wal'
    $rawBefore=(Get-FileHash -LiteralPath $db -Algorithm SHA256).Hash
    $kept=Get-StoreFiles $storeDir
    Assert-True ((@($kept.Keys) -join ',') -ceq 'audit.jsonl,memory.db') "the store is the database and the other files, not -wal/-shm ($(@($kept.Keys) -join ','))"
    Assert-True (Test-Path -LiteralPath "$db-wal") 'reading the store leaves it as it was'
    # Another program opens and closes it: the rows move into memory.db.
    & $python -I -c "import sqlite3,sys;c=sqlite3.connect(sys.argv[1]);c.execute('pragma wal_checkpoint(truncate)');c.close()" $db
    Assert-True ((Get-FileHash -LiteralPath $db -Algorithm SHA256).Hash -cne $rawBefore) 'the checkpoint changed the bytes of memory.db'
    $same=Get-StoreFiles $storeDir
    Assert-True ($same['memory.db'] -ceq $kept['memory.db'] -and $same['audit.jsonl'] -ceq $kept['audit.jsonl']) 'the same memories compare equal'
    & $python -I -c "import sqlite3,sys;c=sqlite3.connect(sys.argv[1]);c.execute('insert into t values (2)');c.commit();c.close()" $db
    Assert-True ((Get-StoreFiles $storeDir)['memory.db'] -cne $kept['memory.db']) 'one more row is a different store'

    # ---- read-routing hooks (on by default): only mmhmem's group may be added.
    $seedSettings=[IO.File]::ReadAllText((Join-Path $script:Root 'ci/e2e/seed/home/.claude/settings.json'))
    $seedHooks=[IO.File]::ReadAllText((Join-Path $script:Root 'ci/e2e/seed/home/.codex/hooks.json'))
    # Claude Code's handler as setup writes it (claude_code.rs): exec form.
    $ours=@{matcher='Read|Bash|PowerShell';hooks=@(@{type='command';command='/home/u/.local/bin/mmhmem';args=@('hook','claude');timeout=10})}
    $s=$seedSettings | ConvertFrom-Json
    $s.hooks.PreToolUse=@($s.hooks.PreToolUse)+@([pscustomobject]$ours)
    $withHook=$s | ConvertTo-Json -Depth 16
    Assert-True ((Compare-ClaudeSettings $seedSettings $withHook $null -HookInstalled).Count -eq 0) 'settings with only the mmhmem hook added pass'
    Assert-True ((Compare-ClaudeSettings $seedSettings $withHook $null).Count -gt 0) 'without -HookInstalled the added hook is a change'
    Assert-True (@(Compare-ClaudeSettings $seedSettings $seedSettings $null -HookInstalled) -match 'no mmhmem read-routing hook').Count -eq 1 'a missing hook is reported'
    $s.hooks.PreToolUse=@($s.hooks.PreToolUse | Where-Object {$_.matcher -cne 'Bash'})
    Assert-True (@(Compare-ClaudeSettings $seedSettings ($s | ConvertTo-Json -Depth 16) $null -HookInstalled) -match 'key hooks changed').Count -eq 1 'a lost user hook is reported'
    $h=$seedHooks | ConvertFrom-Json
    $h.hooks | Add-Member -NotePropertyName PreToolUse -NotePropertyValue @([pscustomobject]@{matcher='^(Bash|shell)$';hooks=@(@{type='command';command='C:\mmhmem\mmhmem.exe hook codex';timeout=10})})
    $codexWithHook=$h | ConvertTo-Json -Depth 16
    Assert-True ((Compare-HooksFile 'hooks.json' $seedHooks $codexWithHook).Count -eq 0) 'Codex hooks with only the mmhmem group added pass'
    Assert-True ((Compare-HooksFile 'hooks.json' $seedHooks $seedHooks).Count -eq 1) 'Codex hooks without the mmhmem group fail'
    $h.hooks.SessionStart[0].hooks[0].command='echo changed'
    Assert-True (@(Compare-HooksFile 'hooks.json' $seedHooks ($h | ConvertTo-Json -Depth 16)) -match 'changed beyond').Count -eq 1 'a changed user hook in Codex fails'
    $check={param($result,$steps) [pscustomobject]@{last_check=[pscustomobject]@{result=$result};pending_user_steps=$steps}}
    Assert-True (Test-AgentCheckOk 'claude_code' (& $check 'pass' @())) 'a pass is OK'
    Assert-True (Test-AgentCheckOk 'codex' (& $check 'user_steps_pending' @('approve_hook_in_codex'))) 'Codex waiting for the hook approval is OK'
    Assert-True (-not (Test-AgentCheckOk 'claude_code' (& $check 'user_steps_pending' @('approve_hook_in_codex')))) 'only Codex may wait'
    Assert-True (-not (Test-AgentCheckOk 'codex' (& $check 'fail' @()))) 'a failed check is not OK'
    Assert-True (-not (Test-AgentCheckOk 'codex' $null)) 'no record entry is not OK'

    # ---- small helpers.
    Assert-True ((ConvertTo-TagPattern 'v0.4.1') -ceq 'v0\.4\.1') 'tag pattern escapes dots only'
    Assert-True (@('windows','linux','macos-aarch64','macos-x86_64') -ccontains (Get-PlatformKey)) 'platform key'
    $one=Get-OneLinerProgram 'http://127.0.0.1:1/v1.0.0'
    if ($script:OnWindows) {
        Assert-True ($one.Args[2] -ceq "irm 'http://127.0.0.1:1/v1.0.0/install.ps1' | iex; exit `$LASTEXITCODE") 'the Windows one-liner in the terminal'
    } else {
        Assert-True ($one.Program -ceq '/bin/sh' -and $one.Args[1] -ceq "curl -fsSL 'http://127.0.0.1:1/v1.0.0/install.sh' | sh") 'the Unix one-liner in the terminal'
    }

    # ---- published-install (e2e-published.ps1): the README one-liners, byte for byte.
    $readme=@([IO.File]::ReadAllText((Join-Path $script:Root 'README.md')) -split "\r?\n")
    foreach ($windows in @($true,$false)) {
        $text=Get-PublishedOneLinerText $windows
        Assert-True ($readme -ccontains $text) "README.md has the published one-liner line '$text'"
    }
    $pw=Get-PublishedOneLiner $true 'C:\shell\powershell.exe' $false
    Assert-True ($pw.File -ceq 'C:\shell\powershell.exe' -and $pw.Arguments -ceq ('-NoProfile -NonInteractive -Command "'+(Get-PublishedOneLinerText $true)+'; exit $LASTEXITCODE"')) 'the published Windows one-liner runs unchanged in the step''s shell'
    $sh=Get-PublishedOneLiner $false '' $true
    Assert-True ($sh.File -ceq 'setsid' -and $sh.Arguments -ceq ('-w /bin/sh -c "'+(Get-PublishedOneLinerText $false)+'"')) 'the published Unix one-liner without a controlling terminal'
    Assert-True ((Get-PublishedOneLiner $false '' $false).File -ceq '/bin/sh') 'without setsid the Unix one-liner runs under /bin/sh'
    $h1='a'*64;$h2='0123456789abcdef'*4
    $sumsText="$h1  mmhmem-v1.0.0-x86_64-pc-windows-msvc.zip`n$h2  mmhmem-v1.0.0-x86_64-unknown-linux-musl.tar.gz`n"
    Assert-True ((Get-ChecksumEntry $sumsText 'mmhmem-v1.0.0-x86_64-unknown-linux-musl.tar.gz') -ceq $h2) 'SHA256SUMS line of one asset'
    Assert-True ((Get-ChecksumEntry ($sumsText -replace "`n","`r`n") 'mmhmem-v1.0.0-x86_64-pc-windows-msvc.zip') -ceq $h1) 'SHA256SUMS with CRLF'
    foreach ($bad in @("$sumsText$h2  mmhmem-v1.0.0-x86_64-unknown-linux-musl.tar.gz`n",'')) {
        $threw=$false;try {$null=Get-ChecksumEntry $bad 'mmhmem-v1.0.0-x86_64-unknown-linux-musl.tar.gz'} catch {$threw=$true}
        Assert-True $threw 'an asset listed twice or not at all is refused'
    }
    $keyBytes=ConvertFrom-HexText '00ff10'
    Assert-True ($keyBytes.Length -eq 3 -and $keyBytes[0] -eq 0 -and $keyBytes[1] -eq 255 -and $keyBytes[2] -eq 16) 'hex to bytes'
    $threw=$false;try {$null=ConvertFrom-HexText 'abc'} catch {$threw=$true}
    Assert-True $threw 'odd-length hex is refused'
    $embedded=@(Get-EmbeddedReleaseKeys)
    Assert-True ($embedded.Count -ge 1 -and (ConvertFrom-HexText $embedded[0]).Length -eq 32) 'the embedded release key is 32 bytes'
    # The program inside an archive, unpacked as the bootstraps do it.
    $payload=Join-Path $work 'payload.bin'
    [IO.File]::WriteAllBytes($payload,[byte[]](1..200 | ForEach-Object {$_ % 256}))
    $payloadHash=(Get-FileHash -LiteralPath $payload -Algorithm SHA256).Hash
    $zip=Join-Path $work 'pkg.zip'
    New-ReleaseZip $payload 'mmhmem.exe' $zip
    $unpacked=Expand-ReleaseEntry $zip 'mmhmem.exe' (Join-Path $work 'unzip')
    Assert-True ((Get-FileHash -LiteralPath $unpacked -Algorithm SHA256).Hash -ceq $payloadHash) 'the program from the zip'
    if (-not $script:OnWindows) {
        $tgz=Join-Path $work 'pkg.tar.gz'
        New-ReleaseTarGz $payload 'mmhmem' $tgz
        $unpacked=Expand-ReleaseEntry $tgz 'mmhmem' (Join-Path $work 'untar')
        Assert-True ((Get-FileHash -LiteralPath $unpacked -Algorithm SHA256).Hash -ceq $payloadHash) 'the program from the tar.gz'
    }
    Write-Host "PASS: e2e harness logic ($script:Checks checks)"
} finally {
    Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
}
