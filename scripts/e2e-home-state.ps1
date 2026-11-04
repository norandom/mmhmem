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
    With -Strict (scenarios where no tool ran: download-404, bad-hash) even
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
    # Codex: logs, sessions, history and caches.
    '.codex/log/*','.codex/sessions/*','.codex/history.jsonl','.codex/version.json',
    '.codex/models_cache.json','.codex/tmp/*','.codex/.tmp/*','.codex/cache/*','.codex/sqlite/*',
    # OpenCode: data, state and cache folders (XDG layout on every OS) and the
    # packages it installs for plugins next to its config.
    '.local/share/opencode/*','.local/state/opencode/*','.cache/opencode/*',
    '.config/opencode/node_modules/*','.config/opencode/package.json',
    '.config/opencode/bun.lock','.config/opencode/.gitignore',
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
    $state=[ordered]@{name=$Name;files=$files;path=$null
        claude_json=$(if (Test-Path -LiteralPath $claude -PathType Leaf) {[IO.File]::ReadAllText($claude)} else {$null})}
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
    return [ordered]@{name=$raw.name;files=$files;path=$raw.path;claude_json=$raw.claude_json}
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
function Compare-ClaudeJson([string]$BeforeText,[string]$AfterText,[ValidateSet('unchanged','installed')][string]$Mode) {
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
    foreach ($key in (Get-JsonNames $before)) {
        if ($key -ceq 'mcpServers' -or $script:ClaudeCounterKeys -ccontains $key) {continue}
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

# ------------------------------------------------------------- compare ---
# Failure messages between two home states (rules in the header).
# -Expected: path patterns this scenario may change, add or remove
# ('HKCU:Path' for the Windows user PATH). -ClaudeMode: see Compare-ClaudeJson.
function Compare-HomeState($Before,$After,[string[]]$Expected=@(),[switch]$Strict,[string]$ClaudeMode='unchanged') {
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
            $claude=Compare-ClaudeJson $Before.claude_json $After.claude_json $ClaudeMode
            if ($claude.Count) {foreach ($f in $claude) {$failures.Add($f)}}
            else {Write-Host '  claude-owned  .claude.json rewritten by Claude Code; every unrelated setting is equal (parsed)'}
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
