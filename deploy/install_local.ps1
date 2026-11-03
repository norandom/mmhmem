# mmhmem local-mode installer (Windows PowerShell; also runs on pwsh).
#
# Registers `mmhmem stdio` as a local MCP server in the harnesses you
# choose and installs the unified-memory playbook — or, with -Remove,
# unregisters it again. No server, no token, no environment variables
# needed (local-mode Req 2.5). The per-user store is user data and is
# NEVER deleted by -Remove; its location is printed instead.
#
# Usage:
#   powershell -File deploy\install_local.ps1                # asks: scope, then harnesses
#   powershell -File deploy\install_local.ps1 -Global        # all projects, every detected harness
#   powershell -File deploy\install_local.ps1 -Global -Agent claude,opencode
#   powershell -File deploy\install_local.ps1 -ProjectDir D:\code\myapp [-Agent LIST]
#   powershell -File deploy\install_local.ps1 -Remove [-Global] [-Agent LIST]
#   powershell -File deploy\install_local.ps1 -Binary PATH -DryRun
#
# Options:
#   -Agent LIST   comma-separated harnesses: claude, opencode, desktop,
#                 antigravity, codex, all (default: interactive selection).
#                 Interactive mode asks for the list when this flag is absent.
#   -WorkerModel NAME   set the shunt worker model - e.g. an Ollama tag like
#                 qwen3:latest. Stored in .mmhmem.json (global or project
#                 per scope); interactive mode offers this when no flag given.
#   -WorkerUrl URL      worker endpoint (default is local Ollama at
#                 http://127.0.0.1:11434/v1/chat/completions).
#   -WorkerApiKey KEY   worker API key for hosted endpoints; stored in
#                 .mmhmem.json, never echoed.
#   -Remove       uninstall: deregister the selected harnesses, remove the
#                 playbook copies and the copied binary. Store and audit
#                 files stay untouched.
#   -DryRun       print actions, change nothing.
#   -Hooks LIST   opt in to the read-routing hook (claude, or all; global
#                 scope only). Shows the exact entry first, adds it to the
#                 user settings file, and leaves Claude Code's own review
#                 (/hooks) to you. A default install adds no hook.
#   -RemoveHooks  remove only the hook entry (everything else stays as it was);
#                 -Remove removes it too.
#
# Behavior:
#   - Global installs copy the binary to a per-user programs dir and
#     register the copy by absolute path, so GUI harnesses (Claude
#     Desktop) can spawn it without a shell PATH.
#   - Per-project installs reference the binary in place; Claude Desktop
#     is app-wide and is skipped in project mode.
#   - Claude Code CLI registers via its own CLI; OpenCode and Claude
#     Desktop configs are JSON-merged in place, preserving existing keys.
#   - Codex registers through ~/.codex/config.toml as a managed
#     [mcp_servers.mmhmem] block (global only; a project install skips it
#     with a note), installs $mmhmem-recap in ~/.agents/skills, and keeps
#     the legacy recap prompt in ~/.codex/prompts for compatibility.
#   - Antigravity imports MCP servers through its settings UI: the script
#     prints the ready-to-import snippet (and the removal step to undo it).
#   - Claude Code and OpenCode additionally get the /mmhmem-recap command
#     template copied into their command directory (scope-matched);
#     -Remove removes it. Desktop and Antigravity have no user-defined
#     commands and get none.

param(
    [switch]$Global,
    [string]$ProjectDir,
    [string]$Binary,
    [string]$InstallSource,
    [string]$Agent,
    [string]$WorkerModel,
    [string]$WorkerUrl,
    [string]$WorkerApiKey,
    [string]$Hooks,
    [switch]$RemoveHooks,
    [switch]$Remove,
    [switch]$DryRun,
    [switch]$NonInteractive
)

$ErrorActionPreference = 'Stop'

# Interactivity: prompts only make sense on a real terminal. When stdin is
# redirected (CI, script callers, this harness) the flagless invocation
# must fail fast instead of hanging on Read-Host.
$Interactive = -not $NonInteractive -and -not [Console]::IsInputRedirected

function Info([string]$msg) { Write-Host "[mmhmem] $msg" }
function Die([string]$msg) { Write-Host "install_local.ps1: $msg" -ForegroundColor Red; exit 1 }
# PS 5.1 quirk: redirected native stderr can raise under ErrorActionPreference
# Stop; a best-effort deregistration must never abort the installer.
function Remove-ClaudeServer([string[]]$ClaudeArgs) {
    $old = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try { & claude @ClaudeArgs 2>$null | Out-Null } catch {} finally { $ErrorActionPreference = $old }
}

# --- read-routing hook (opt-in; hooks_onboarding.ps1) ------------------------------
$Exe = if ($IsWindows -or ($env:OS -eq 'Windows_NT')) { 'mmhmem.exe' } else { 'mmhmem' }
# Per-user programs dir: inside the Windows apps convention, else ~/.local/bin.
$UserBinDir = if ($env:LOCALAPPDATA) { Join-Path $env:LOCALAPPDATA 'Programs\mmhmem' } else { Join-Path $HOME '.local\bin' }
$UserBin = Join-Path $UserBinDir $Exe
. (Join-Path $PSScriptRoot 'hooks_onboarding.ps1')
$HookSource = Join-Path $PSScriptRoot 'manage_hooks.ps1'
$HookDestination = Join-Path $UserBinDir 'manage_hooks.ps1'
try { $HookHarnesses = @(Get-HookHarnesses $Hooks) } catch { Die $_.Exception.Message }
if ($HookHarnesses.Count -gt 0 -and $RemoveHooks) { Die '-Hooks and -RemoveHooks are mutually exclusive' }
if ($RemoveHooks -and $Remove) { Die '-Remove already removes the hook entry; use -RemoveHooks alone for hook-only removal' }
if ($HookHarnesses.Count -gt 0 -and $Remove) { Die '-Hooks cannot be combined with -Remove' }
$HookProblem = $false
if ($RemoveHooks -and -not $Remove) {
    # Hook-only removal: nothing else is read, installed or removed.
    $code = Remove-ReadRoutingHook $HookSource $HookDestination -DryRun:$DryRun
    if ($DryRun) { Info 'dry run - nothing was written; re-run without -DryRun to apply' }
    exit $code
}

# --- agent selection ---------------------------------------------------------
$Agents = @()
if ($Agent) {
    $Agents = ($Agent -split ',') | ForEach-Object { $_.Trim().ToLower() } | Where-Object { $_ }
    $valid = @('claude', 'opencode', 'desktop', 'antigravity', 'codex', 'all')
    foreach ($a in $Agents) {
        if ($valid -notcontains $a) { Die "unknown agent '$a' (valid: claude, opencode, desktop, antigravity, codex, all)" }
    }
}
function Want-Agent([string]$name) {
    if ($Agents.Count -eq 0) { return $true }
    if ($Agents -contains 'all') { return $true }
    return $Agents -contains $name
}
function Agent-Requested([string]$name) {
    if ($Agents.Count -eq 0 -or $Agents -contains 'all') { return $false }
    return Want-Agent $name
}
if (-not $Agent -and -not $Interactive) { Die 'pass -Agent with a comma-separated list, or -Agent all (no terminal for selection)' }

# --- scope --------------------------------------------------------------------
if ($Global -and $ProjectDir) { Die '-Global and -ProjectDir are mutually exclusive' }
if (-not $Global -and -not $ProjectDir) {
    if (-not $Interactive) { Die 'pass -Global or -ProjectDir (stdin is not a terminal)' }
    while ($true) {
        $ans = Read-Host @"
Install mmhmem local mode:
  [g] globally for all projects (user scope)
  [p] for this project only ($PWD)
Choose (g/p)
"@
        switch -Regex ($ans) {
            '^(?i)(g|global|all|a)$' { $Global = [switch]$true; break }
            '^(?i)(p|project|t|this)$' { $ProjectDir = (Get-Location).Path; break }
        }
        if ($Global -or $ProjectDir) { break }
    }
}
if ($ProjectDir -and -not (Test-Path -LiteralPath $ProjectDir -PathType Container)) {
    Die "project dir not found: $ProjectDir"
}

# --- worker settings (.mmhmem.json; corpus-warmup-prune Req 7.6) ----------------
# worker_model is the activation switch: without it the worker stays off and
# shunting/warmup is disabled; the URL defaults to local Ollama, so an Ollama
# user only needs the model name here. Settings are stored in .mmhmem.json -
# the global one for user scope, the project one for project scope - and the
# binary reads them at startup; the harness registrations carry no worker env.
# The file is JSON-merged (other keys preserved); the API key is never echoed.
# Scope-selected worker-config file (project scope: the installer's project
# dir, which the spawned child also uses as its working directory).
$WorkerConfigFile = if ($Remove) { $null }
    elseif ($Global) { Join-Path $HOME '.mmhmem.json' }
    elseif ($ProjectDir) { Join-Path $ProjectDir '.mmhmem.json' }
    else { $null }
if (-not $Remove) {
    . (Join-Path $PSScriptRoot 'check_worker.ps1')
    $ExistingWorker = Get-MmhmemWorkerSettings $WorkerConfigFile $WorkerModel $WorkerUrl $WorkerApiKey
    Show-MmhmemWorkerSettings $ExistingWorker $WorkerConfigFile
}
# JSON-quotes one scalar value (plain model names/tags/URLs pass through).
# NB: .NET regex replacements do not process backslash escapes, so the
# replacement strings here are literal '\\' (two chars) and '\"'.
function Json-Quote([string]$v) {
    return '"' + (($v -replace '\\', '\\') -replace '"', '\"') + '"'
}
# Writes only the provided settings into the config file (JSON-merged).
function Write-WorkerConfig {
    if ($WorkerModel) { Merge-Json $WorkerConfigFile @('worker_model') (Json-Quote $WorkerModel) }
    if ($WorkerUrl) { Merge-Json $WorkerConfigFile @('worker_api_url') (Json-Quote $WorkerUrl) }
    if ($WorkerApiKey) { Merge-Json $WorkerConfigFile @('worker_api_key') (Json-Quote $WorkerApiKey) }
}

# --- binary -------------------------------------------------------------------
$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if ($Remove) {
    # Removal only needs the standard copy location (global installs).
    $Bin = $UserBin
} elseif ($Binary) {
    $Bin = if ($DryRun -or $InstallSource) { [IO.Path]::GetFullPath($Binary) } else { (Resolve-Path -LiteralPath $Binary).Path }
} elseif (Get-Command mmhmem -ErrorAction SilentlyContinue) {
    $Bin = (Get-Command mmhmem).Source
} elseif (Test-Path -LiteralPath (Join-Path $RepoRoot "target\release\$Exe")) {
    $Bin = (Resolve-Path -LiteralPath (Join-Path $RepoRoot "target\release\$Exe")).Path
} else {
    Die "mmhmem binary not found - build it (cargo build --release) or pass -Binary PATH"
}

$SkillSrc = Join-Path $RepoRoot 'deploy\skills\unified-memory\SKILL.md'
if (-not $Remove -and -not (Test-Path -LiteralPath $SkillSrc)) { Die "SKILL.md not found at $SkillSrc" }
function Install-Skill([string]$dir, [string]$src = $SkillSrc) {
    if ($DryRun) { Info "[dry-run] copy SKILL.md -> $dir\SKILL.md"; return }
    New-Item -ItemType Directory -Path $dir -Force | Out-Null
    Copy-Item -LiteralPath $src -Destination (Join-Path $dir 'SKILL.md') -Force
    Info "playbook installed: $dir\SKILL.md"
}
function Remove-Skill([string]$dir) {
    if ($DryRun) { Info "[dry-run] rm $dir\SKILL.md"; return }
    $skillFile = Join-Path $dir 'SKILL.md'
    if (Test-Path -LiteralPath $skillFile) { Remove-Item -LiteralPath $skillFile -Force }
    if ((Test-Path -LiteralPath $dir) -and -not (Get-ChildItem -LiteralPath $dir -Force)) {
        Remove-Item -LiteralPath $dir -Force
    }
    Info "playbook removed: $dir"
}
function Set-Skill([string]$dir) {
    if ($Remove) { Remove-Skill $dir } else { Install-Skill $dir }
}

# /mmhmem-recap command templates (corpus-warmup-prune Req 9): copied per
# scope into the harness's own command directory; desktop/antigravity have
# no user-defined commands and get none.
$CommandSrcClaude = Join-Path $RepoRoot 'deploy\commands\claude\mmhmem-recap.md'
$CommandSrcOpenCode = Join-Path $RepoRoot 'deploy\commands\opencode\mmhmem-recap.md'
function Install-Command([string]$src, [string]$dst) {
    if ($DryRun) { Info "[dry-run] copy $(Split-Path -Leaf $src) -> $dst"; return }
    New-Item -ItemType Directory -Path (Split-Path -Parent $dst) -Force | Out-Null
    Copy-Item -LiteralPath $src -Destination $dst -Force
    Info "command installed: $dst"
}
function Remove-Command([string]$dst) {
    if ($DryRun) { Info "[dry-run] rm $dst"; return }
    if (Test-Path -LiteralPath $dst) { Remove-Item -LiteralPath $dst -Force }
    Info "command removed: $dst"
}
function Set-Command([string]$src, [string]$dst) {
    if ($Remove) { Remove-Command $dst } else { Install-Command $src $dst }
}

# Merges one JSON assignment into a config file, preserving everything
# else in it. $File = path, $JsonPath = "a.b.c", $ValueJson = raw JSON.
function Merge-Json([string]$File, [string[]]$JsonPath, [string]$ValueJson) {
    if ($DryRun) { Info "[dry-run] merge $($JsonPath -join '.') into $File"; return }
    New-Item -ItemType Directory -Path (Split-Path -Parent $File) -Force | Out-Null
    $value = $ValueJson | ConvertFrom-Json
    if (Test-Path -LiteralPath $File) {
        $root = Get-Content -LiteralPath $File -Raw | ConvertFrom-Json
    } else {
        $root = New-Object PSObject
    }
    if (-not $root) { $root = New-Object PSObject }
    $node = $root
    for ($i = 0; $i -lt $JsonPath.Count - 1; $i++) {
        $name = $JsonPath[$i]
        if (-not $node.PSObject.Properties[$name]) {
            $node | Add-Member -MemberType NoteProperty -Name $name -Value (New-Object PSObject)
        }
        $node = $node.$name
    }
    $node | Add-Member -MemberType NoteProperty -Name $JsonPath[-1] -Value $value -Force
    # BOM-less UTF8: Windows PowerShell's Set-Content -Encoding UTF8 writes a
    # BOM, which strict JSON parsers (including mmhmem's own loader) reject.
    $json = $root | ConvertTo-Json -Depth 10
    [System.IO.File]::WriteAllText($File, $json, (New-Object System.Text.UTF8Encoding($false)))
}

# Removes a JSON key from a config file. $File = path, $JsonPath = "a.b.c".
function Remove-JsonKey([string]$File, [string[]]$JsonPath) {
    if ($DryRun) { Info "[dry-run] remove $($JsonPath -join '.') from $File"; return }
    if (-not (Test-Path -LiteralPath $File)) { return }
    $root = Get-Content -LiteralPath $File -Raw | ConvertFrom-Json
    if (-not $root) { return }
    $node = $root
    for ($i = 0; $i -lt $JsonPath.Count - 1; $i++) {
        $name = $JsonPath[$i]
        if (-not $node.PSObject.Properties[$name]) { return }
        $node = $node.$name
    }
    if ($node.PSObject.Properties[$JsonPath[-1]]) {
        $node.PSObject.Properties.Remove($JsonPath[-1])
        $json = $root | ConvertTo-Json -Depth 10
        [System.IO.File]::WriteAllText($File, $json, (New-Object System.Text.UTF8Encoding($false)))
    }
}

# --- harness detection + interactive selection -----------------------------------
$HasClaude = [bool](Get-Command claude -ErrorAction SilentlyContinue)
$OpenCodeGlobalCfg = Join-Path $HOME '.config\opencode\opencode.json'
$HasOpenCode = ((Get-Command opencode -ErrorAction SilentlyContinue) -ne $null) -or (Test-Path -LiteralPath $OpenCodeGlobalCfg)
$DesktopCfgDir = if ($env:APPDATA) { Join-Path $env:APPDATA 'Claude' } else { Join-Path $HOME 'Library/Application Support/Claude' }
# MSIX virtualizes Claude's roaming data; prefer that live profile over a legacy path.
if ($env:LOCALAPPDATA) {
    $packages = Join-Path $env:LOCALAPPDATA 'Packages'
    if (Test-Path -LiteralPath $packages) {
        $desktopProfiles = @(Get-ChildItem -LiteralPath $packages -Directory -Filter 'Claude_*' | ForEach-Object {
            $profile = Join-Path $_.FullName 'LocalCache\Roaming\Claude'
            if (Test-Path -LiteralPath $profile -PathType Container) { $profile }
        })
        if ($desktopProfiles.Count -gt 1) { Die 'Multiple Claude Desktop package profiles found; cannot choose a config safely.' }
        if ($desktopProfiles.Count -eq 1) { $DesktopCfgDir = $desktopProfiles[0] }
    }
}
$HasDesktop = Test-Path -LiteralPath $DesktopCfgDir -PathType Container
$HasAntigravity = ((Get-Command antigravity -ErrorAction SilentlyContinue) -ne $null) -or (Test-Path -LiteralPath (Join-Path $HOME '.gemini'))
$CodexCfgDir = Join-Path $HOME '.codex'
$HasCodex = ((Get-Command codex -ErrorAction SilentlyContinue) -ne $null) -or (Test-Path -LiteralPath $CodexCfgDir)
$CodexCommandSrc = Join-Path $RepoRoot 'deploy\commands\codex\mmhmem-recap.md'

# Codex registers through ~/.codex/config.toml (global, TOML). The section
# is a managed block between marker comments so no TOML parser is needed:
# install replaces the block, remove strips it.
$CodexBegin = '# BEGIN mmhmem (managed by install_local.ps1 -- do not edit)'
$CodexEnd = '# END mmhmem'
function Codex-Block([string]$bin) {
    $esc = $bin.Replace('\', '\\')
    "$CodexBegin`n[mcp_servers.mmhmem]`ncommand = `"$esc`"`nargs = [`"stdio`"]`n$CodexEnd"
}
function Get-CodexUnmanagedLines([string[]]$lines) {
    $keep = New-Object 'System.Collections.Generic.List[string]'
    $managed = $false
    foreach ($line in $lines) {
        if ($line -match '^# BEGIN mmhmem \(managed by install_local\.(ps1|sh) -- do not edit\)$') {
            if ($managed) { throw 'Nested mmhmem markers; Codex config unchanged.' }
            $managed = $true; continue
        }
        if ($line -eq '# END mmhmem') {
            if (-not $managed) { throw 'Unmatched mmhmem marker; Codex config unchanged.' }
            $managed = $false; continue
        }
        if ($line.Contains('"""') -or $line.Contains(([string][char]39 * 3))) { throw 'Multiline TOML needs manual review; Codex config unchanged.' }
        if ($managed) {
            if ($line -notmatch '^\s*(#.*|\[mcp_servers\.mmhmem\]|command\s*=.*|args\s*=.*)?\s*$') { throw 'Custom managed mmhmem settings need manual review; Codex config unchanged.' }
            continue
        }
        $normal = $line -replace '[\s"''\[\]]', ''
        if ($normal -match '^mcp_servers\.mmhmem([.#=]|$)' -or $normal -match '^mcp_servers=' -or $normal -match '^mmhmem([.#=]|$)') {
            throw 'Existing unmanaged mmhmem registration needs manual review; Codex config unchanged.'
        }
        $keep.Add($line)
    }
    if ($managed) { throw 'Unclosed mmhmem marker; Codex config unchanged.' }
    return ,$keep.ToArray()
}
function Write-CodexRegistration([string]$cfg, [string[]]$lines) {
    New-Item -ItemType Directory -Path (Split-Path -Parent $cfg) -Force | Out-Null
    $temporary = $cfg + '.' + [guid]::NewGuid().ToString('N') + '.new'
    try {
        [IO.File]::WriteAllText($temporary, (($lines -join "`n") + "`n"), (New-Object Text.UTF8Encoding($false)))
        if (Test-Path -LiteralPath $cfg) { [IO.File]::Replace($temporary, $cfg, ($cfg + '.mmhmem.bak')) }
        else { [IO.File]::Move($temporary, $cfg) }
    } finally { if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary } }
}
function Set-CodexRegistration([string]$bin) {
    $cfg = Join-Path $CodexCfgDir 'config.toml'
    if ($DryRun) { Info "[dry-run] merge [mcp_servers.mmhmem] into $cfg"; return }
    $lines = if (Test-Path -LiteralPath $cfg) { @(Get-Content -LiteralPath $cfg -Encoding UTF8) } else { @() }
    $keep = Get-CodexUnmanagedLines $lines
    Write-CodexRegistration $cfg @($keep + (Codex-Block $bin))
    Info "codex registration written: $cfg"
}
function Remove-CodexRegistration() {
    $cfg = Join-Path $CodexCfgDir 'config.toml'
    if ($DryRun) { Info "[dry-run] remove [mcp_servers.mmhmem] from $cfg"; return }
    if (-not (Test-Path -LiteralPath $cfg)) { return }
    $lines = @(Get-Content -LiteralPath $cfg -Encoding UTF8)
    $keep = Get-CodexUnmanagedLines $lines
    Write-CodexRegistration $cfg $keep
    Info "codex registration removed: $cfg"
}

if (-not $Agent -and $Interactive) {
    $menu = New-Object System.Collections.Generic.List[string]
    $keys = New-Object System.Collections.Generic.List[string]
    if ($HasClaude) { $menu.Add('  [1] claude - Claude Code CLI'); $keys.Add('claude') }
    if ($HasOpenCode) { $menu.Add('  [2] opencode - OpenCode'); $keys.Add('opencode') }
    if ($HasDesktop -and $Global) { $menu.Add('  [3] desktop - Claude Desktop'); $keys.Add('desktop') }
    if ($HasAntigravity) { $menu.Add('  [4] antigravity - print the import snippet'); $keys.Add('antigravity') }
    if ($HasCodex) { $menu.Add('  [5] codex - OpenAI Codex CLI'); $keys.Add('codex') }
    if ($menu.Count -eq 0) { Die 'No supported agents detected. Install an agent, then run this installer again.' }
    # Number only the entries actually displayed (detection may leave gaps).
    for ($i = 0; $i -lt $menu.Count; $i++) { $menu[$i] = $menu[$i] -replace '\[\d+\]', ('[' + ($i + 1) + ']') }
    if ($menu.Count -gt 0) {
        while ($true) {
            $verb = if ($Remove) { 'Remove integrations from' } else { 'Connect' }
            $prompt = "`n  $verb your agents`n  ----------------------------------------`n" + ($menu -join "`n") + "`n  [a] All listed`n  [q] Cancel`n`n  Choose one or more (e.g. 1,3)"
            $ans = Read-Host $prompt
            $ans = ($ans -replace '\s', '')
            if (-not $ans) { continue }
            if ($ans -eq 'q') { exit 0 }
            if ($ans -match '^(?i)a$') { $Agents = @('all'); break }
            $nums = $ans -split ','
            $picked = @()
            $ok = $true
            foreach ($n in $nums) {
                if ($n -match '^[1-5]$' -and [int]$n -le $keys.Count) {
                    $picked += $keys[[int]$n - 1]
                } else { $ok = $false; break }
            }
            if ($ok -and $picked.Count -gt 0) { $Agents = $picked; break }
            Write-Host '  Please choose numbers from the list, a, or q.' -ForegroundColor Yellow
        }
    }
}

$Registered = New-Object System.Collections.Generic.List[string]
$MissingRequested = New-Object System.Collections.Generic.List[string]
function Note-Missing([string]$key, [bool]$detected, [string]$doc) {
    if (-not (Want-Agent $key)) { return }
    if (-not $detected) {
        if ((Agent-Requested $key) -and -not $Remove) {
            $MissingRequested.Add($key) | Out-Null
        } else {
            Info "$doc not found on this machine - nothing to do for it"
        }
    }
}
Note-Missing 'claude' $HasClaude 'Claude Code CLI'
Note-Missing 'opencode' $HasOpenCode 'OpenCode'
Note-Missing 'desktop' $HasDesktop 'Claude Desktop'
Note-Missing 'antigravity' $HasAntigravity 'Antigravity'
Note-Missing 'codex' $HasCodex 'Codex CLI'
if ($MissingRequested.Count -gt 0) { Die ("requested but not detected: " + ($MissingRequested -join ' ')) }
# Refuse ambiguous Codex edits before committing binary or worker changes.
if ($Global -and (Want-Agent 'codex') -and $HasCodex -and -not $DryRun) {
    $cfg = Join-Path $CodexCfgDir 'config.toml'
    if (Test-Path -LiteralPath $cfg) { $null = Get-CodexUnmanagedLines @(Get-Content -LiteralPath $cfg -Encoding UTF8) }
}

# Validate every selected routing destination before any harness/config writes.
. (Join-Path $PSScriptRoot 'routing_onboarding.ps1')
$RoutingPlans = @{}
foreach ($harness in @('claude','opencode')) {
    $detected = if ($harness -eq 'claude') { $HasClaude } else { $HasOpenCode }
    if ((Want-Agent $harness) -and $detected) {
        $routingProject = if ($ProjectDir) { $ProjectDir } else { $HOME }
        $RoutingPlans[$harness] = New-HarnessRoutingPlan -Harness $harness -UserRoot $HOME -Project $routingProject -Global:$Global -Remove:$Remove
    }
}

# --- read-routing hook opt-in (before any write; nothing is added by default) ---------
$HookClaude = $HookHarnesses -contains 'claude'
$HookCandidate = (-not $Remove) -and $Global -and (Want-Agent 'claude') -and $HasClaude
if ($HookClaude -and -not $HookCandidate) {
    Die '-Hooks claude needs a global install (-Global) with the claude agent selected and detected; no hook was added'
}
if ($HookCandidate -and ($HookClaude -or ($Interactive -and -not $Hooks))) {
    Write-Host "`n  Optional: Claude Code read-routing hook`n  ----------------------------------------"
    if ((Show-ReadRoutingHook $HookSource $UserBin $HookDestination) -ne 0) { Die 'cannot show the read-routing hook; no hook was added' }
    if (-not $HookClaude) {
        $addHook = Read-Host '  Add this hook to Claude Code? [y/N]'
        $HookClaude = $addHook -match '^(?i)y(es)?$'
    }
    if ($HookClaude -and -not (Test-ReadRoutingHookAllowed $HookSource $UserBin)) {
        Die 'the read-routing hook was not added and nothing was changed'
    }
}

# --- optional worker setup -------------------------------------------------
if (-not $Remove -and -not $WorkerModel -and $Interactive) {
    Write-Host "`n  Optional: summary cache`n  ----------------------------------------"
    Write-Host "  A worker model prepares reusable file summaries.`n  Memory works without it."
    Write-Host "  Keep existing settings by answering N.`n  This does not download or start a model."
    $configureWorker = Read-Host '  Configure a worker model now? [y/N]'
    if ($configureWorker -match '^(?i)y(es)?$') {
        $modelPrompt = if ($ExistingWorker.Model) { "  Model ID [$($ExistingWorker.Model)]" } else { '  Model ID (empty = skip)' }
        $WorkerModel = (Read-Host $modelPrompt).Trim()
        if (-not $WorkerModel -and $ExistingWorker) { $WorkerModel = $ExistingWorker.Model }
        if ($WorkerModel) {
            $defaultBase = $ExistingWorker.Url -replace '/chat/completions/?$', ''
            if (-not $defaultBase) { $defaultBase = 'http://127.0.0.1:11434/v1' }
            $safeDefault = $null
            if ([Uri]::TryCreate([string]$ExistingWorker.Url, [UriKind]::Absolute, [ref]$safeDefault)) {
                $defaultBase = $safeDefault.GetLeftPart([UriPartial]::Path) -replace '/chat/completions/?$', '' -replace '//[^/@]+@', '//'
            }
            Write-Host '  Press Enter to keep each bracketed value.'
            $enteredUrl = (Read-Host "  OpenAI-compatible base URL [$defaultBase]").Trim().TrimEnd('/')
            if ($enteredUrl) {
                $WorkerUrl = if ($enteredUrl.EndsWith('/chat/completions')) { $enteredUrl } else { "$enteredUrl/chat/completions" }
            }
            if (-not $WorkerApiKey) {
                Write-Host '  Empty = keep existing key; local Ollama needs no key.'
                $keyLabel = if ($ExistingWorker.Key) { '***' } else { 'not set' }
                $secureKey = Read-Host "  API key [$keyLabel] (hidden)" -AsSecureString
                $keyPointer = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secureKey)
                try { $WorkerApiKey = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($keyPointer) }
                finally { [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($keyPointer) }
            }
        }
    }
}
# --- end optional worker setup ---------------------------------------------
if (-not $Remove) {
    if ($DryRun) { Info '[dry-run] worker connection check skipped (no API request)' }
    else { Test-MmhmemWorker (Get-MmhmemWorkerSettings $WorkerConfigFile $WorkerModel $WorkerUrl $WorkerApiKey) }
}
if ($InstallSource -and -not $Remove -and -not $DryRun) {
    # Commit the staged bootstrap download only after the user selects agents.
    . (Join-Path $PSScriptRoot 'install_binary.ps1')
    Install-MmhmemBinary -Source $InstallSource -Destination $Bin
}

# --- worker config file (before registrations; they stay worker-env-free) -----
# Req 7.6: the installers write .mmhmem.json instead of embedding worker env.
# Only provided settings are written; other keys in the file are preserved.
$WorkerConfigWritten = $false
if (-not $Remove -and $WorkerConfigFile -and ($WorkerModel -or $WorkerUrl -or $WorkerApiKey)) {
    Write-WorkerConfig
    $WorkerConfigWritten = $true
    Info "worker settings stored in $WorkerConfigFile (provided settings written; other keys preserved)"
}

# --- Claude Code CLI ---------------------------------------------------------------
if ((Want-Agent 'claude') -and $HasClaude) {
    if ($Remove) {
        if ($Global) {
            if ($DryRun) { Info "[dry-run] claude mcp remove mmhmem -s user" }
            else { Remove-ClaudeServer @('mcp', 'remove', 'mmhmem', '-s', 'user') }
        } elseif ($ProjectDir) {
            Push-Location -LiteralPath $ProjectDir
            try {
                if ($DryRun) { Info "[dry-run] claude mcp remove mmhmem (in $ProjectDir)" }
                else { Remove-ClaudeServer @('mcp', 'remove', 'mmhmem') }
            } finally { Pop-Location }
        }
    } elseif ($Global) {
        if (-not $DryRun) { Remove-ClaudeServer @('mcp', 'remove', 'mmhmem', '-s', 'user') }
        if ($DryRun) {
            Info "[dry-run] claude mcp add --transport stdio --scope user mmhmem -- $Bin stdio"
        } else {
            claude mcp add --transport stdio --scope user mmhmem -- $Bin stdio
        }
    } else {
        Push-Location -LiteralPath $ProjectDir
        try {
            if (-not $DryRun) { Remove-ClaudeServer @('mcp', 'remove', 'mmhmem') }
            if ($DryRun) {
                Info "[dry-run] claude mcp add --transport stdio mmhmem -- $Bin stdio (in $ProjectDir)"
            } else {
                claude mcp add --transport stdio mmhmem -- $Bin stdio
            }
        } finally { Pop-Location }
    }
    Set-Skill (Join-Path $HOME '.claude\skills\unified-memory')
    $claudeCmdDir = if ($Global) { Join-Path $HOME '.claude\commands' } else { Join-Path $ProjectDir '.claude\commands' }
    Set-Command $CommandSrcClaude (Join-Path $claudeCmdDir 'mmhmem-recap.md')
    Complete-HarnessRouting -Plan $RoutingPlans['claude'] -DryRun:$DryRun
    $Registered.Add('claude-code') | Out-Null
}

# --- OpenCode -----------------------------------------------------------------------
if ((Want-Agent 'opencode') -and $HasOpenCode) {
    $cfg = if ($Global) { $OpenCodeGlobalCfg } else { Join-Path $ProjectDir 'opencode.json' }
    if ($Remove) {
        Remove-JsonKey $cfg @('mcp', 'servers', 'mmhmem')
    } else {
        Merge-Json $cfg @('mcp', 'servers', 'mmhmem') `
            "{`"type`": `"local`", `"command`": [$(Json-Quote $Bin), `"stdio`"]}"
    }
    if ($Global) { Set-Skill (Join-Path $HOME '.config\opencode\skills\unified-memory') }
    $openCodeCmdDir = if ($Global) { Join-Path $HOME '.config\opencode\command' } else { Join-Path $ProjectDir '.opencode\command' }
    Set-Command $CommandSrcOpenCode (Join-Path $openCodeCmdDir 'mmhmem-recap.md')
    Complete-HarnessRouting -Plan $RoutingPlans['opencode'] -DryRun:$DryRun
    $Registered.Add('opencode') | Out-Null
}

# --- Claude Desktop (global scope only) ------------------------------------------------
if ((Want-Agent 'desktop') -and $HasDesktop) {
    if ($Global) {
        $deskCfg = Join-Path $DesktopCfgDir 'claude_desktop_config.json'
        if ($Remove) {
            Remove-JsonKey $deskCfg @('mcpServers', 'mmhmem')
        } else {
            Merge-Json $deskCfg @('mcpServers', 'mmhmem') `
                "{`"command`": $(Json-Quote $Bin), `"args`": [`"stdio`"]}"
        }
        Set-Skill (Join-Path $DesktopCfgDir 'skills\unified-memory')
        $Registered.Add('claude-desktop') | Out-Null
    } elseif (-not $Remove) {
        Info "Claude Desktop is app-wide; skipped for a per-project install (run again with -Global)"
    }
}

# --- Antigravity (manual import; no public config path) --------------------------------
if ((Want-Agent 'antigravity') -and $HasAntigravity) {
    if ($Remove) {
        Info "Antigravity: remove the mmhmem server in Antigravity Settings > MCP (added manually)"
    } else {
        Info "Antigravity: add a stdio MCP server in Settings with this JSON:"
        Write-Host "  { `"mcpServers`": { `"mmhmem`": { `"command`": $(Json-Quote $Bin), `"args`": [`"stdio`"] } } }"
    }
    $Registered.Add('antigravity-manual') | Out-Null
}

# --- OpenAI Codex CLI (global config.toml; managed block) -------------------------------
if ((Want-Agent 'codex') -and $HasCodex) {
    if ($Remove) {
        Remove-CodexRegistration
        Remove-Command (Join-Path $CodexCfgDir 'prompts\mmhmem-recap.md')
        Remove-Skill (Join-Path $HOME '.agents\skills\unified-memory')
        Remove-Skill (Join-Path $HOME '.agents\skills\mmhmem-recap')
        $Registered.Add('codex') | Out-Null
    } elseif ($Global) {
        Set-CodexRegistration $Bin
        Install-Skill (Join-Path $HOME '.agents\skills\unified-memory')
        Install-Skill (Join-Path $HOME '.agents\skills\mmhmem-recap') (Join-Path $RepoRoot 'deploy\codex\skills\mmhmem-recap\SKILL.md')
        Set-Command $CodexCommandSrc (Join-Path $CodexCfgDir 'prompts\mmhmem-recap.md')
        $Registered.Add('codex') | Out-Null
    } else {
        Info "Codex reads a global config only; skipped for a per-project install (run again with -Global)"
    }
}

# --- copied binary (global installs) -----------------------------------------------------
if ($Global) {
    if ($Remove) {
        if ($Agents -notcontains 'all') { Info 'Shared binary kept for other agents; use -Remove -Agent all to remove it.' }
        elseif (Test-Path -LiteralPath $UserBin) {
            if ($DryRun) { Info "[dry-run] rm $UserBin" }
            else {
                try {
                    Remove-Item -LiteralPath $UserBin -Force
                    Info "removed binary copy: $UserBin"
                } catch {
                    Info "binary copy is in use (a harness may be running): $UserBin - delete it manually later"
                }
            }
        }
    } elseif ($Bin -ne $UserBin) {
        if ($DryRun) { Info "[dry-run] copy $Bin -> $UserBin" }
        else {
            . (Join-Path $PSScriptRoot 'install_binary.ps1')
            Install-MmhmemBinary -Source $Bin -Destination $UserBin
        }
    }
}

# --- read-routing hook (after the binary copy it points to) -----------------------------------
if ($Remove -and $Global -and (Want-Agent 'claude')) {
    if ((Remove-ReadRoutingHook $HookSource $HookDestination -DryRun:$DryRun) -ne 0) {
        $HookProblem = $true
        Info 'the read-routing hook entry could not be removed; see the message above and remove it by hand'
    }
} elseif ($HookClaude) {
    if ((Add-ReadRoutingHook $HookSource $HookDestination $UserBin -DryRun:$DryRun) -eq 0) { Write-HookAdvisory }
    else { $HookProblem = $true; Info 'the read-routing hook was not added; see the message above' }
} elseif ($HookCandidate) {
    Info 'read-routing hook not added (opt-in): re-run with -Hooks claude to review and add it'
}

# --- summary ---------------------------------------------------------------------------------
if ($Global -and -not $Remove) {
    if ($DryRun) { Info "[dry-run] add $UserBinDir to user PATH" }
    else {
        $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        $entries = @($userPath -split ';' | Where-Object { $_ })
        if (@($entries | Where-Object { $_.TrimEnd('\') -ieq $UserBinDir.TrimEnd('\') }).Count -eq 0) {
            [Environment]::SetEnvironmentVariable('Path', (($entries + $UserBinDir) -join ';'), 'User')
        }
        Info 'mmhmem is on your user PATH. New terminals will pick it up.'
    }
}
if ($Registered.Count -eq 0 -and -not $WorkerConfigWritten) { Die 'nothing to do: no harness selected/detected' }
Write-Host ''
$Mode = if ($Global) { 'global (all projects)' } else { "project: $ProjectDir" }
if ($Remove) {
    Info "mmhmem local mode removed ($Mode)"
    Info ("harnesses: " + ($Registered -join ' '))
    Info "per-user store left intact (it is your data): %APPDATA%\mmhmem - delete it manually if you really want"
    Info ".mmhmem.json kept (it may hold your worker settings and API key)"
} else {
    Info "mmhmem local mode installed ($Mode)"
    Info ("harnesses: " + ($Registered -join ' '))
    Info "binary: $Bin"
    if ($WorkerModel) {
        Info "worker model: $WorkerModel (shunting and warmup active; stored in $WorkerConfigFile)"
    } else {
        Info "worker settings unchanged; summaries and warmup require worker_model in $WorkerConfigFile or WORKER_MODEL in the environment"
    }
    Info "store: per-user default (%APPDATA%\mmhmem); created on first use"
    Info "check your savings any time: $Bin stats"
}
if ($DryRun) { Info 'dry run - nothing was written; re-run without -DryRun to apply' }
if ($HookProblem) { exit 1 }
if (-not $Remove -and $Interactive) {
    Write-Host "`n  Optional: explore and prebuild your project cache"
    $warmup = Read-Host '  Show the prompt to run in your agent? [y/N]'
    if ($warmup -match '^(?i)y(es)?$') {
        Write-Host "  Open the project in your agent.`n  Restart its mmhmem connection to load new settings."
        Write-Host "  A reachable worker model is required.`n  Remote providers may charge for summaries."
        Write-Host "`n  Paste this prompt into your agent:`n"
        Write-Host "  Explore this repository and identify its key source files.`n  Use mmhmem shunt_warmup to prebuild their summary cache`n  in batches of up to 25 files.`n  Report what was cached or skipped."
    }
}
