#Requires -Version 5.1
<#
.SYNOPSIS
Home snapshots and comparisons for the installer end-to-end job
(scripts/e2e-run.ps1, reliable-installers task 8.2, Requirement 10.3/10.4).
.DESCRIPTION
Dot-sourced by e2e-run.ps1 (needs $script:SeedHome, $script:Manifest,
$script:WorkDirectory, $script:OnWindows, $script:OnMac, Get-SeedFiles,
Read-SeedManifest and Get-UserPathSnapshot). Runs nothing and changes nothing
outside the work folder.

A home state is every file under the e2e home with its SHA-256 (Windows: the
home's Temp folder is left out, the bootstrap's own temporary folder lives
there and is checked separately), the HKCU\Environment\Path value on Windows,
and the text of ~/.claude.json.

Comparison rules (Compare-HomeState), strict by default:
  - every seeded file, everything mmhmem owns (program, install record,
    memory store) and every other file must be byte-identical, present or
    absent exactly as before, unless the caller lists it as expected to
    change for that scenario;
  - ~/.claude.json is the one exception. Claude Code rewrites that whole file
    itself (its own formatting and bookkeeping keys) whenever its CLI runs,
    for example during the `claude mcp get mmhmem` check, so after any run
    that may have started `claude` its bytes cannot be compared. It is then
    compared parsed (Compare-ClaudeJson): mcpServers exactly, with mmhmem's
    entry only where the scenario expects it; every other seeded key and
    every seeded project entry unchanged except Claude's own counters
    ($script:ClaudeCounterKeys); keys Claude adds must not mention mmhmem.
    The first time it runs, Claude also moves legacy settings out of
    ~/.claude.json into ~/.claude/settings.json (Get-ClaudeMoves); that file
    is then compared parsed too (Compare-ClaudeSettings): everything as
    before plus exactly those moves. With -Strict (scenarios where no tool ran: download-404, bad-hash) even
    ~/.claude.json must be byte-identical;
  - on Windows, the shell's own caches under %LOCALAPPDATA% / %APPDATA%
    ($script:ShellRuntimePatterns) may appear even with -Strict;
  - without -Strict, files that are not seeded and lie in the agent CLIs' own
    runtime folders ($script:AgentRuntimePatterns: logs, caches, session
    state that `claude`, `codex` and `opencode` write when setup detects or
    checks them) may appear or change. They are listed in the log. A seeded
    file never falls under this rule.
#>

# Agent CLIs' own runtime files: never seeded, never written by mmhmem. Each
# pattern is matched case-sensitively against the '/'-separated path
# relative to the home (PowerShell -clike, '*' also matches '/').
$script:AgentRuntimePatterns=@(
    # Claude Code: feature-flag cache, todos, shell snapshots, per-project
    # session logs, debug logs, IDE locks, plugin cache and its own backups
    # of ~/.claude.json.
    '.claude/statsig/*','.claude/todos/*','.claude/shell-snapshots/*','.claude/projects/*',
    '.claude/debug/*','.claude/session-env/*','.claude/ide/*','.claude/plugins/*',
    '.claude/backups/*','.claude.json.backup*',
    # Claude Code's log of its connection to the mmhmem server, written by
    # `claude mcp get mmhmem` in its own cache folder (env-paths layout:
    # Linux, macOS, Windows; research.md section 8, run 41994007615).
    '.cache/claude-cli-nodejs/*/mcp-logs-mmhmem/*.jsonl',
    'Library/Caches/claude-cli-nodejs/*/mcp-logs-mmhmem/*.jsonl',
    'AppData/Local/claude-cli-nodejs/Cache/*/mcp-logs-mmhmem/*.jsonl',
    # Codex: logs, sessions, history and caches.
    '.codex/log/*','.codex/sessions/*','.codex/history.jsonl','.codex/version.json',
    '.codex/models_cache.json','.codex/tmp/*','.codex/.tmp/*','.codex/cache/*','.codex/sqlite/*',
    # OpenCode: data, state and cache folders (XDG layout on every OS) and the
    # packages it installs for plugins next to its config.
    '.local/share/opencode/*','.local/state/opencode/*','.cache/opencode/*',
    '.config/opencode/node_modules/*','.config/opencode/package.json',
    '.config/opencode/bun.lock','.config/opencode/.gitignore',
    # The password of OpenCode's background service, created by OpenCode
    # when the service first starts (setup's `opencode mcp list` check) and
    # kept after `opencode service stop` (research.md section 8).
    '.config/opencode/service.json',
    # npm / npx caches the CLIs' launchers may touch.
    '.npm/*','AppData/Local/npm-cache/*'
)
# The Windows shell's own files: the one-liner runs in powershell.exe or
# pwsh, which keep startup and module caches (and .NET usage logs) under
# %LOCALAPPDATA% / %APPDATA%, both inside the e2e home. They belong to the
# shell the user typed into, not to the installer, so they may appear even
# in a strict comparison; a seeded file never falls under this rule.
$script:ShellRuntimePatterns=@(
    'AppData/Local/Microsoft/Windows/PowerShell/*','AppData/Local/Microsoft/PowerShell/*',
    'AppData/Roaming/Microsoft/Windows/PowerShell/*','AppData/Local/Microsoft/CLR_v4.0*',
    'AppData/Local/Microsoft/Windows/INetCache/*','AppData/Local/Microsoft/Windows/INetCookies/*'
)
# Top-level ~/.claude.json keys that Claude Code itself counts up.
$script:ClaudeCounterKeys=@('numStartups','tipsHistory')
# Legacy ~/.claude.json settings Claude Code moves into ~/.claude/settings.json
# the first time its CLI runs (2.1.311, reproduced in a throwaway home,
# research.md section 8): "autoUpdates": false is removed and becomes
# env.DISABLE_AUTOUPDATER "1"; "theme" stays and is copied. These moves, and
# no others, are Claude's (Get-ClaudeMoves).
function Get-ClaudeMoves($ClaudeJson) {
    $moves=@{removed=@{};env=@{};copied=@{}}
    if ($null -eq $ClaudeJson) {return $moves}
    $auto=Get-JsonMember $ClaudeJson 'autoUpdates'
    if ($auto[0] -and $auto[1] -is [bool] -and -not $auto[1]) {$moves.removed['autoUpdates']=$true;$moves.env['DISABLE_AUTOUPDATER']='1'}
    $theme=Get-JsonMember $ClaudeJson 'theme'
    if ($theme[0]) {$moves.copied['theme']=$theme[1]}
    return $moves
}

# Where mmhmem keeps its things, relative to the home ('/'-separated).
function Get-MmhmemRelativePaths {
    if ($script:OnWindows) {
        return @{Record='AppData/Local/mmhmem/install.json';RecordDir='AppData/Local/mmhmem'
            Program='AppData/Local/Programs/mmhmem/mmhmem.exe';DataDir='AppData/Roaming/mmhmem'}
    }
    if ($script:OnMac) {
        return @{Record='Library/Application Support/mmhmem/install.json';RecordDir='Library/Application Support/mmhmem'
            Program='.local/bin/mmhmem';DataDir='Library/Application Support/mmhmem'}
    }
    return @{Record='.local/state/mmhmem/install.json';RecordDir='.local/state/mmhmem'
        Program='.local/bin/mmhmem';DataDir='.local/share/mmhmem'}
}

function Test-StateExcluded([string]$Rel) {
    return ($script:OnWindows -and $Rel -clike 'AppData/Local/Temp/*')
}
function Test-AnyLike([string]$Rel,[string[]]$Patterns) {
    foreach ($p in $Patterns) {if ($p -and $Rel -clike $p) {return $true}}
    return $false
}

# The state of the home now; also saved to snapshots/<Name>.json for the logs.
function Get-HomeState([string]$Name) {
    $files=[ordered]@{}
    foreach ($rel in (Get-SeedFiles $script:SeedHome)) {
        if (Test-StateExcluded $rel) {continue}
        try {$files[$rel]=(Get-FileHash -LiteralPath (Join-Path $script:SeedHome $rel) -Algorithm SHA256 -ErrorAction Stop).Hash.ToLowerInvariant()}
        catch {$files[$rel]='unreadable'}
    }
    $claude=Join-Path $script:SeedHome '.claude.json'
    $settings=Join-Path $script:SeedHome '.claude/settings.json'
    $state=[ordered]@{name=$Name;files=$files;path=$null
        claude_json=$(if (Test-Path -LiteralPath $claude -PathType Leaf) {[IO.File]::ReadAllText($claude)} else {$null})
        claude_settings=$(if (Test-Path -LiteralPath $settings -PathType Leaf) {[IO.File]::ReadAllText($settings)} else {$null})}
    if ($script:OnWindows) {$state.path=Get-UserPathSnapshot}
    $dir=Join-Path $script:WorkDirectory 'snapshots'
    New-Item -ItemType Directory -Path $dir -Force | Out-Null
    [IO.File]::WriteAllText((Join-Path $dir "$Name.json"),(($state | ConvertTo-Json -Depth 6)+"`n"),(New-Object Text.UTF8Encoding($false)))
    return $state
}
function Read-HomeState([string]$Name) {
    $path=Join-Path (Join-Path $script:WorkDirectory 'snapshots') "$Name.json"
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {throw "No home snapshot '$Name' (run the earlier phase first)"}
    $raw=[IO.File]::ReadAllText($path) | ConvertFrom-Json
    $files=[ordered]@{}
    foreach ($p in $raw.files.PSObject.Properties) {$files[$p.Name]=[string]$p.Value}
    return [ordered]@{name=$raw.name;files=$files;path=$raw.path;claude_json=$raw.claude_json;claude_settings=$raw.claude_settings}
}

# ------------------------------------------------------- canonical JSON ---
# One text per JSON value with object keys sorted (ordinal), so two parsed
# documents are equal exactly when their canonical texts are.
function ConvertTo-CanonicalJson($Value) {
    if ($null -eq $Value) {return 'null'}
    if ($Value -is [bool]) {if ($Value) {return 'true'} else {return 'false'}}
    if ($Value -is [string]) {return (ConvertTo-Json -InputObject $Value -Compress)}
    if ($Value -is [datetime]) {return (ConvertTo-Json -InputObject $Value.ToString('o',[Globalization.CultureInfo]::InvariantCulture) -Compress)}
    if ($Value -is [ValueType]) {return [Convert]::ToString($Value,[Globalization.CultureInfo]::InvariantCulture)}
    if ($Value -is [Collections.IDictionary]) {
        [string[]]$keys=@($Value.Keys | ForEach-Object {[string]$_})
        [Array]::Sort($keys,[StringComparer]::Ordinal)
        $parts=foreach ($k in $keys) {(ConvertTo-Json -InputObject $k -Compress)+':'+(ConvertTo-CanonicalJson $Value[$k])}
        return '{'+(@($parts) -join ',')+'}'
    }
    if ($Value -is [Management.Automation.PSCustomObject]) {
        [string[]]$keys=@($Value.PSObject.Properties | ForEach-Object {$_.Name})
        [Array]::Sort($keys,[StringComparer]::Ordinal)
        $parts=foreach ($k in $keys) {(ConvertTo-Json -InputObject $k -Compress)+':'+(ConvertTo-CanonicalJson $Value.$k)}
        return '{'+(@($parts) -join ',')+'}'
    }
    if ($Value -is [Collections.IEnumerable]) {
        $parts=New-Object Collections.Generic.List[string]
        foreach ($item in $Value) {$parts.Add((ConvertTo-CanonicalJson $item))}
        return '['+($parts -join ',')+']'
    }
    return (ConvertTo-Json -InputObject ([string]$Value) -Compress)
}
# Property names of a parsed JSON object (empty for anything else).
function Get-JsonNames($Object) {
    if ($Object -isnot [Management.Automation.PSCustomObject]) {return @()}
    return @($Object.PSObject.Properties | ForEach-Object {$_.Name})
}
function Get-JsonMember($Object,[string]$Name) {
    if ((Get-JsonNames $Object) -ccontains $Name) {return ,@($true,$Object.$Name)}
    return ,@($false,$null)
}

# ------------------------------------------------------- ~/.claude.json ---
# Failure messages comparing two ~/.claude.json texts (see the header).
# -Mode unchanged: mmhmem's server entry must be as before (both absent, or
# equal); installed: it must be present now. Every other server entry, every
# seeded key except the counters, and every seeded project must be equal.
# A key Claude moves (Get-ClaudeMoves) may be gone only when -AfterSettings
# (the ~/.claude/settings.json text now) holds it.
function Compare-ClaudeJson([string]$BeforeText,[string]$AfterText,[ValidateSet('unchanged','installed')][string]$Mode,[string]$AfterSettings) {
    $failures=New-Object Collections.Generic.List[string]
    if ([string]::IsNullOrEmpty($AfterText)) {$failures.Add('~/.claude.json is missing or empty');return ,$failures.ToArray()}
    try {$before=$BeforeText | ConvertFrom-Json} catch {$failures.Add('the earlier ~/.claude.json is not JSON');return ,$failures.ToArray()}
    try {$after=$AfterText | ConvertFrom-Json} catch {$failures.Add('~/.claude.json is no longer valid JSON');return ,$failures.ToArray()}
    $bServers=(Get-JsonMember $before 'mcpServers')[1]
    $aServers=(Get-JsonMember $after 'mcpServers')[1]
    if ($Mode -ceq 'installed' -and -not (Get-JsonMember $aServers 'mmhmem')[0]) {$failures.Add('~/.claude.json has no mcpServers.mmhmem')}
    $names=@(@(Get-JsonNames $bServers)+@(Get-JsonNames $aServers) | Select-Object -Unique)
    foreach ($n in $names) {
        $b=Get-JsonMember $bServers $n;$a=Get-JsonMember $aServers $n
        if ($n -ceq 'mmhmem' -and $Mode -ceq 'installed') {continue}
        if ($b[0] -ne $a[0]) {$failures.Add("~/.claude.json mcpServers.$n was $(if ($a[0]) {'added'} else {'removed'})");continue}
        if ((ConvertTo-CanonicalJson $b[1]) -cne (ConvertTo-CanonicalJson $a[1])) {$failures.Add("~/.claude.json mcpServers.$n changed")}
    }
    $afterNames=Get-JsonNames $after
    $moves=Get-ClaudeMoves $before
    foreach ($key in (Get-JsonNames $before)) {
        if ($key -ceq 'mcpServers' -or $script:ClaudeCounterKeys -ccontains $key) {continue}
        if ($afterNames -cnotcontains $key -and $moves.removed.ContainsKey($key)) {
            $settingsEnv=$null
            if ($AfterSettings) {try {$settingsEnv=(Get-JsonMember ($AfterSettings | ConvertFrom-Json) 'env')[1]} catch {}}
            $moved=@($moves.env.Keys | Where-Object {(Get-JsonMember $settingsEnv $_)[1] -cne $moves.env[$_]}).Count -eq 0
            if ($moved) {Write-Host "  claude-owned: key $key moved by Claude Code into ~/.claude/settings.json"}
            else {$failures.Add("~/.claude.json lost the key $key (it is not in ~/.claude/settings.json either)")}
            continue
        }
        if ($afterNames -cnotcontains $key) {$failures.Add("~/.claude.json lost the key $key");continue}
        if ($key -ceq 'projects') {
            foreach ($p in (Get-JsonNames $before.projects)) {
                $a=Get-JsonMember $after.projects $p
                if (-not $a[0] -or (ConvertTo-CanonicalJson $before.projects.$p) -cne (ConvertTo-CanonicalJson $a[1])) {$failures.Add("~/.claude.json projects[$p] changed")}
            }
            foreach ($p in (Get-JsonNames $after.projects)) {
                if ((Get-JsonNames $before.projects) -ccontains $p) {continue}
                if ((ConvertTo-CanonicalJson $after.projects.$p) -match '(?i)mmhmem') {$failures.Add("~/.claude.json projects[$p] (added by Claude Code) mentions mmhmem")}
                else {Write-Host "  claude-owned: projects[$p] added by Claude Code"}
            }
            continue
        }
        if ((ConvertTo-CanonicalJson $before.$key) -cne (ConvertTo-CanonicalJson $after.$key)) {$failures.Add("~/.claude.json key $key changed")}
    }
    foreach ($key in $afterNames) {
        if ((Get-JsonNames $before) -ccontains $key) {continue}
        if ((ConvertTo-CanonicalJson $after.$key) -match '(?i)mmhmem') {$failures.Add("~/.claude.json key $key (added by Claude Code) mentions mmhmem")}
        else {Write-Host "  claude-owned: key $key added by Claude Code"}
    }
    return ,$failures.ToArray()
}

# ---------------------------------------------- ~/.claude/settings.json ---
# Failure messages comparing two ~/.claude/settings.json texts, after Claude
# Code ran with -ClaudeJson (the ~/.claude.json text before it ran). Every
# key and every env entry must be as before (parsed: Claude rewrites the
# formatting); the only additions allowed are Claude's moves out of
# ~/.claude.json (Get-ClaudeMoves), each with exactly the moved value.
function Compare-ClaudeSettings([string]$BeforeText,[string]$AfterText,[string]$ClaudeJson,[switch]$HookInstalled) {
    $failures=New-Object Collections.Generic.List[string]
    if ([string]::IsNullOrEmpty($AfterText)) {$failures.Add('~/.claude/settings.json is missing or empty');return ,$failures.ToArray()}
    try {$before=$BeforeText | ConvertFrom-Json} catch {$failures.Add('the earlier ~/.claude/settings.json is not JSON');return ,$failures.ToArray()}
    try {$after=$AfterText | ConvertFrom-Json} catch {$failures.Add('~/.claude/settings.json is no longer valid JSON');return ,$failures.ToArray()}
    if ($HookInstalled) {
        $stripped=Remove-MmhmemHookHandlers $after
        if (-not $stripped.found) {$failures.Add('~/.claude/settings.json has no mmhmem read-routing hook')}
        $after=$stripped.json
    }
    $moves=Get-ClaudeMoves $(if ($ClaudeJson) {try {$ClaudeJson | ConvertFrom-Json} catch {$null}} else {$null})
    $bEnv=(Get-JsonMember $before 'env')[1];$aEnv=(Get-JsonMember $after 'env')[1]
    foreach ($n in (Get-JsonNames $bEnv)) {
        $a=Get-JsonMember $aEnv $n
        if (-not $a[0] -or (ConvertTo-CanonicalJson $bEnv.$n) -cne (ConvertTo-CanonicalJson $a[1])) {$failures.Add("~/.claude/settings.json env.$n changed")}
    }
    foreach ($n in (Get-JsonNames $aEnv)) {
        if ((Get-JsonNames $bEnv) -ccontains $n) {continue}
        if ($moves.env.ContainsKey($n) -and $aEnv.$n -ceq $moves.env[$n]) {Write-Host "  claude-owned: settings env.$n moved by Claude Code from ~/.claude.json"}
        else {$failures.Add("~/.claude/settings.json env.$n was added")}
    }
    $afterNames=Get-JsonNames $after
    foreach ($key in (Get-JsonNames $before)) {
        if ($key -ceq 'env') {continue}
        if ($afterNames -cnotcontains $key) {$failures.Add("~/.claude/settings.json lost the key $key");continue}
        if ((ConvertTo-CanonicalJson $before.$key) -cne (ConvertTo-CanonicalJson $after.$key)) {$failures.Add("~/.claude/settings.json key $key changed")}
    }
    foreach ($key in $afterNames) {
        if ($key -ceq 'env' -or (Get-JsonNames $before) -ccontains $key) {continue}
        if ($moves.copied.ContainsKey($key) -and (ConvertTo-CanonicalJson $after.$key) -ceq (ConvertTo-CanonicalJson $moves.copied[$key])) {Write-Host "  claude-owned: settings key $key copied by Claude Code from ~/.claude.json"}
        else {$failures.Add("~/.claude/settings.json key $key was added")}
    }
    return ,$failures.ToArray()
}

# ------------------------------------------------- read-routing hooks ---
# The read-routing hook is on by default (setup flow_select.rs), so an
# install adds one handler running `<mmhmem> hook <harness>` to the agent's
# hooks file. mmhmem owns that HANDLER, never its group: setup adds it in a
# group of its own, but one it finds inside a user's group (old installs,
# adoption) stays there (src/setup/agents/hook_entry.rs). These helpers take
# exactly mmhmem's handlers out, so every other handler, group, event and key
# can still be compared with the seed.
$script:MmhmemHookCommand='(?i)mmhmem(\.exe)?["''‘’‚‛]?\s+hook\s+(claude|codex|opencode)\b'
# @{json=<parsed copy without mmhmem's handlers>; found=<whether one was there>}.
# A group left without handlers is dropped, and then an event array left
# without groups.
function Remove-MmhmemHookHandlers($Json) {
    $copy=($Json | ConvertTo-Json -Depth 32) | ConvertFrom-Json
    $found=$false
    $hooks=(Get-JsonMember $copy 'hooks')[1]
    if ($null -eq $hooks) {return @{json=$copy;found=$false}}
    foreach ($event in @(Get-JsonNames $hooks)) {
        $groups=@($hooks.$event)
        $kept=@()
        $changed=$false
        foreach ($group in $groups) {
            $handlers=@($group.hooks)
            # Claude Code's handler is in exec form (command = the binary,
            # args = hook claude); Codex's is one command string.
            $others=@($handlers | Where-Object {((@([string]$_.command)+@($_.args | ForEach-Object {[string]$_})) -join ' ') -notmatch $script:MmhmemHookCommand})
            if ($others.Count -eq $handlers.Count) {$kept+=,$group;continue}
            $found=$true;$changed=$true
            if ($others.Count) {$group.hooks=$others;$kept+=,$group}
        }
        if (-not $changed) {continue}
        if ($kept.Count) {$hooks.$event=$kept} else {$hooks.PSObject.Properties.Remove($event)}
    }
    return @{json=$copy;found=$found}
}
# Failure messages for a hooks file (Codex ~/.codex/hooks.json) after an
# install: parsed, it must equal the earlier text plus mmhmem's handler.
function Compare-HooksFile([string]$Label,[string]$BeforeText,[string]$AfterText) {
    $failures=New-Object Collections.Generic.List[string]
    try {$before=$BeforeText | ConvertFrom-Json} catch {$failures.Add("the earlier $Label is not JSON");return ,$failures.ToArray()}
    try {$after=$AfterText | ConvertFrom-Json} catch {$failures.Add("$Label is no longer valid JSON");return ,$failures.ToArray()}
    $stripped=Remove-MmhmemHookHandlers $after
    if (-not $stripped.found) {$failures.Add("$Label has no mmhmem read-routing hook")}
    if ((ConvertTo-CanonicalJson $before) -cne (ConvertTo-CanonicalJson $stripped.json)) {$failures.Add("$Label changed beyond the mmhmem hook")}
    return ,$failures.ToArray()
}
# Whether an agent's check result is acceptable after an install: a pass,
# or for Codex with its hook on, only the approval in /hooks left to do.
function Test-AgentCheckOk([string]$Agent,$Entry) {
    if (-not $Entry -or -not $Entry.last_check) {return $false}
    $result=[string]$Entry.last_check.result
    if ($result -ceq 'pass') {return $true}
    $pending=@($Entry.pending_user_steps | ForEach-Object {[string]$_})
    return ($Agent -ceq 'codex' -and $result -ceq 'user_steps_pending' -and $pending.Count -eq 1 -and $pending[0] -ceq 'approve_hook_in_codex')
}

# ------------------------------------------------------------- compare ---
# Failure messages between two home states (rules in the header).
# -Expected: path patterns this scenario may change, add or remove
# ('HKCU:Path' for the Windows user PATH). -ClaudeMode: see Compare-ClaudeJson.
function Compare-HomeState($Before,$After,[string[]]$Expected=@(),[switch]$Strict,[string]$ClaudeMode='unchanged',[switch]$HooksInstalled) {
    $seeded=Read-SeedManifest $script:Manifest
    $failures=New-Object Collections.Generic.List[string]
    $all=New-Object 'Collections.Generic.SortedSet[string]' ([StringComparer]::Ordinal)
    foreach ($k in $Before.files.Keys) {$null=$all.Add($k)}
    foreach ($k in $After.files.Keys) {$null=$all.Add($k)}
    $same=0
    foreach ($rel in $all) {
        $b=$Before.files[$rel];$a=$After.files[$rel]
        if ($b -ceq $a) {$same++;continue}
        $kind=if (-not $b) {'new'} elseif (-not $a) {'removed'} else {'changed'}
        if (Test-AnyLike $rel $Expected) {Write-Host "  expected-$kind  $rel";continue}
        if (-not $Strict -and $rel -ceq '.claude.json' -and $b -and $a) {
            $claude=Compare-ClaudeJson $Before.claude_json $After.claude_json $ClaudeMode $After.claude_settings
            if ($claude.Count) {foreach ($f in $claude) {$failures.Add($f)}}
            else {Write-Host '  claude-owned  .claude.json rewritten by Claude Code; every unrelated setting is equal (parsed)'}
            continue
        }
        if (-not $Strict -and $rel -ceq '.claude/settings.json' -and $b -and $a) {
            $claude=Compare-ClaudeSettings $Before.claude_settings $After.claude_settings $Before.claude_json -HookInstalled:$HooksInstalled
            if ($claude.Count) {foreach ($f in $claude) {$failures.Add($f)}}
            else {Write-Host '  claude-owned  .claude/settings.json rewritten by Claude Code; only its own moves (parsed)'}
            continue
        }
        if ($HooksInstalled -and $rel -ceq '.codex/hooks.json' -and $b -and $a) {
            # Compared with the seeded text: the states keep hashes only, and
            # every scenario starts from the seeded hooks file.
            $path=Join-Path $script:SeedHome $rel
            $seedText=[IO.File]::ReadAllText((Join-Path $script:Root "ci/e2e/seed/home/$rel"))
            $hookFailures=Compare-HooksFile '~/.codex/hooks.json' $seedText ([IO.File]::ReadAllText($path))
            if ($hookFailures.Count) {foreach ($f in $hookFailures) {$failures.Add($f)}}
            else {Write-Host '  expected-hook  .codex/hooks.json gained only the mmhmem hook (parsed)'}
            continue
        }
        if (-not $seeded.Contains($rel) -and $script:OnWindows -and (Test-AnyLike $rel $script:ShellRuntimePatterns)) {Write-Host "  shell-own-$kind  $rel";continue}
        if (-not $Strict -and -not $seeded.Contains($rel) -and (Test-AnyLike $rel $script:AgentRuntimePatterns)) {Write-Host "  agent-own-$kind  $rel";continue}
        $failures.Add("$kind file: $rel")
    }
    if ($script:OnWindows) {
        $bp=$Before.path;$ap=$After.path
        if ([string]$bp.exists -ne [string]$ap.exists -or [string]$bp.kind -cne [string]$ap.kind -or [string]$bp.value -cne [string]$ap.value) {
            if ($Expected -ccontains 'HKCU:Path') {Write-Host '  expected-changed  HKCU\Environment\Path'}
            else {$failures.Add('HKCU\Environment\Path changed')}
        } else {$same++}
    }
    Write-Host "compare $($Before.name) -> $($After.name): $same unchanged, $($failures.Count) failure(s)"
    return ,$failures.ToArray()
}

# The bootstrap's temporary folders that are still there (it must always
# remove them): Windows %TEMP% (inside the home), else $TMPDIR.
function Get-BootstrapLeftovers {
    $temp=if ($script:OnWindows) {Join-Path $script:SeedHome 'AppData\Local\Temp'} else {Join-Path $script:WorkDirectory 'tmp'}
    if (-not (Test-Path -LiteralPath $temp)) {return @()}
    return @(Get-ChildItem -LiteralPath $temp -Force -Directory | Where-Object {$_.Name -like 'mmhmem-install*'} | ForEach-Object {$_.FullName})
}

# ---------------------------------------------------- leftover programs ---
# Programs that must not be left running once setup has finished: mmhmem
# itself (`mmhmem stdio`, started by a check or by an agent for it) and
# OpenCode's background service (`opencode serve --service`), which setup's
# OpenCode check starts through `opencode mcp list` and stops again when it
# was not running before (research.md section 8). A leftover mmhmem keeps the
# memory store open (on Windows nothing else can read memory.db then) and the
# program file in use. Rows: objects with Id, Name and CommandLine.
function Select-LeftoverProcesses($Rows) {
    return @($Rows | Where-Object {
        ([string]$_.Name -match '^mmhmem(\.exe)?$') -or
        ([string]$_.CommandLine -match '(?i)opencode\S*\s+serve\s+(\S+\s+)*--service(\s|$)')
    } | ForEach-Object {"pid $($_.Id): $($_.CommandLine)"})
}
# Every process of this machine as rows for Select-LeftoverProcesses.
function Get-ProcessRows {
    if ($script:OnWindows) {
        return @(Get-CimInstance Win32_Process | ForEach-Object {
            [pscustomobject]@{Id=$_.ProcessId;Name=$_.Name;CommandLine=$(if ($_.CommandLine) {$_.CommandLine} else {$_.Name})}
        })
    }
    $rows=foreach ($line in @(& ps -A -o pid= -o args= 2>$null)) {
        if ([string]$line -match '^\s*(\d+)\s+(.+)$') {
            $command=$Matches[2]
            [pscustomobject]@{Id=[int]$Matches[1];Name=((($command -split '\s+')[0]) -split '/')[-1];CommandLine=$command}
        }
    }
    return @($rows)
}
# What is still running after up to -Seconds (a program may need a moment
# to end after setup exits). One pipeline item per process, none when all
# ended: callers pipe it into ForEach-Object.
function Wait-NoLeftoverProcesses([int]$Seconds=15) {
    for ($i=0;;$i++) {
        $left=@(Select-LeftoverProcesses (Get-ProcessRows))
        if (-not $left.Count -or $i -ge 2*$Seconds) {return $left}
        Start-Sleep -Milliseconds 500
    }
}
