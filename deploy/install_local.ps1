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
#                 antigravity, all (default: all detected). Interactive
#                 mode asks for the list when this flag is absent.
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
#
# Behavior:
#   - Global installs copy the binary to a per-user programs dir and
#     register the copy by absolute path, so GUI harnesses (Claude
#     Desktop) can spawn it without a shell PATH.
#   - Per-project installs reference the binary in place; Claude Desktop
#     is app-wide and is skipped in project mode.
#   - Claude Code CLI registers via its own CLI; OpenCode and Claude
#     Desktop configs are JSON-merged in place, preserving existing keys.
#   - Antigravity imports MCP servers through its settings UI: the script
#     prints the ready-to-import snippet (and the removal step to undo it).

param(
    [switch]$Global,
    [string]$ProjectDir,
    [string]$Binary,
    [string]$Agent,
    [string]$WorkerModel,
    [string]$WorkerUrl,
    [string]$WorkerApiKey,
    [switch]$Remove,
    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'

# Interactivity: prompts only make sense on a real terminal. When stdin is
# redirected (CI, script callers, this harness) the flagless invocation
# must fail fast instead of hanging on Read-Host.
$Interactive = -not [Console]::IsInputRedirected

function Info([string]$msg) { Write-Host "[mmhmem] $msg" }
function Die([string]$msg) { Write-Host "install_local.ps1: $msg" -ForegroundColor Red; exit 1 }
# PS 5.1 quirk: redirected native stderr can raise under ErrorActionPreference
# Stop; a best-effort deregistration must never abort the installer.
function Remove-ClaudeServer([string[]]$ClaudeArgs) {
    $old = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try { & claude @ClaudeArgs 2>$null | Out-Null } catch {} finally { $ErrorActionPreference = $old }
}

# --- agent selection ---------------------------------------------------------
$Agents = @()
if ($Agent) {
    $Agents = ($Agent -split ',') | ForEach-Object { $_.Trim().ToLower() } | Where-Object { $_ }
    $valid = @('claude', 'opencode', 'desktop', 'antigravity', 'all')
    foreach ($a in $Agents) {
        if ($valid -notcontains $a) { Die "unknown agent '$a' (valid: claude, opencode, desktop, antigravity, all)" }
    }
}
function Want-Agent([string]$name) {
    if ($Agents.Count -eq 0) { return $true }
    if ($Agents -contains 'all') { return $true }
    return $Agents -contains $name
}
function Agent-Requested([string]$name) {
    if ($Agents.Count -eq 0) { return $false }
    return Want-Agent $name
}

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
if (-not $Remove -and -not $WorkerModel -and $Interactive) {
    $WorkerModel = Read-Host 'Worker model for shunting (e.g. qwen3:latest via local Ollama; empty = skip, shunting stays off)'
}
# Scope-selected worker-config file (project scope: the installer's project
# dir, which the spawned child also uses as its working directory).
$WorkerConfigFile = if ($Remove) { $null }
    elseif ($Global) { Join-Path $HOME '.mmhmem.json' }
    elseif ($ProjectDir) { Join-Path $ProjectDir '.mmhmem.json' }
    else { $null }
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
$Exe = if ($IsWindows -or ($env:OS -eq 'Windows_NT')) { 'mmhmem.exe' } else { 'mmhmem' }
# Per-user programs dir: inside the Windows apps convention, else ~/.local/bin.
$UserBinDir = if ($env:LOCALAPPDATA) { Join-Path $env:LOCALAPPDATA 'Programs\mmhmem' } else { Join-Path $HOME '.local\bin' }
$UserBin = Join-Path $UserBinDir $Exe
if ($Remove) {
    # Removal only needs the standard copy location (global installs).
    $Bin = $UserBin
} elseif ($Binary) {
    $Bin = (Resolve-Path -LiteralPath $Binary).Path
} elseif (Get-Command mmhmem -ErrorAction SilentlyContinue) {
    $Bin = (Get-Command mmhmem).Source
} elseif (Test-Path -LiteralPath (Join-Path $RepoRoot "target\release\$Exe")) {
    $Bin = (Resolve-Path -LiteralPath (Join-Path $RepoRoot "target\release\$Exe")).Path
} else {
    Die "mmhmem binary not found - build it (cargo build --release) or pass -Binary PATH"
}

$SkillSrc = Join-Path $RepoRoot 'deploy\skills\unified-memory\SKILL.md'
if (-not (Test-Path -LiteralPath $SkillSrc)) { Die "SKILL.md not found at $SkillSrc (run from a repo checkout)" }
function Install-Skill([string]$dir) {
    if ($DryRun) { Info "[dry-run] copy SKILL.md -> $dir\SKILL.md"; return }
    New-Item -ItemType Directory -Path $dir -Force | Out-Null
    Copy-Item -LiteralPath $SkillSrc -Destination (Join-Path $dir 'SKILL.md') -Force
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
$HasDesktop = Test-Path -LiteralPath $DesktopCfgDir -PathType Container
$HasAntigravity = ((Get-Command antigravity -ErrorAction SilentlyContinue) -ne $null) -or (Test-Path -LiteralPath (Join-Path $HOME '.gemini'))

if (-not $Agent -and $Interactive -and $Host.Name -eq 'ConsoleHost') {
    $menu = New-Object System.Collections.Generic.List[string]
    $keys = New-Object System.Collections.Generic.List[string]
    if ($HasClaude) { $menu.Add('  [1] claude - Claude Code CLI'); $keys.Add('claude') }
    if ($HasOpenCode) { $menu.Add('  [2] opencode - OpenCode'); $keys.Add('opencode') }
    if ($HasDesktop -and $Global) { $menu.Add('  [3] desktop - Claude Desktop'); $keys.Add('desktop') }
    if ($HasAntigravity) { $menu.Add('  [4] antigravity - print the import snippet'); $keys.Add('antigravity') }
    if ($menu.Count -gt 0) {
        while ($true) {
            $prompt = "Which harness(es)?`n" + ($menu -join "`n") + "`n  [a] all listed`nChoose (comma-separated numbers, or a)"
            $ans = Read-Host $prompt
            $ans = ($ans -replace '\s', '')
            if (-not $ans) { continue }
            if ($ans -match '^(?i)a$') { $Agents = @(); break }
            $nums = $ans -split ','
            $picked = @()
            $ok = $true
            foreach ($n in $nums) {
                if ($n -match '^\d+$' -and [int]$n -ge 1 -and [int]$n -le $keys.Count) {
                    $picked += $keys[[int]$n - 1]
                } else { $ok = $false; break }
            }
            if ($ok -and $picked.Count -gt 0) { $Agents = $picked; break }
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
if ($MissingRequested.Count -gt 0) { Die ("requested but not detected: " + ($MissingRequested -join ' ')) }

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

# --- copied binary (global installs) -----------------------------------------------------
if ($Global) {
    if ($Remove) {
        if (Test-Path -LiteralPath $UserBin) {
            if ($DryRun) { Info "[dry-run] rm $UserBin" }
            else {
                Remove-Item -LiteralPath $UserBin -Force
                Info "removed binary copy: $UserBin"
            }
        }
    } elseif ($Bin -ne $UserBin) {
        if ($DryRun) { Info "[dry-run] copy $Bin -> $UserBin" }
        else {
            New-Item -ItemType Directory -Path $UserBinDir -Force | Out-Null
            Copy-Item -LiteralPath $Bin -Destination $UserBin -Force
        }
    }
}

# --- summary ---------------------------------------------------------------------------------
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
        Info "worker model not configured - shunting and warmup stay off; re-run with -WorkerModel NAME (e.g. an Ollama tag) or set worker_model in $WorkerConfigFile"
    }
    Info "store: per-user default (%APPDATA%\mmhmem); created on first use"
    Info "check your savings any time: $Bin stats"
}
if ($DryRun) { Info 'dry run - nothing was written; re-run without -DryRun to apply' }
