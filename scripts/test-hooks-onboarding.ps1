# Installer opt-in for the read-routing hook (Windows). Every path is a scratch override:
# CLAUDE_CONFIG_DIR, LOCALAPPDATA and $HOME (patched into a disposable engine copy) all point
# below tmp/, so the real ~/.claude/settings.json and the user PATH are never touched.
$ErrorActionPreference = 'Stop'
$root = Join-Path (Split-Path $PSScriptRoot -Parent) ('tmp/hooks-onboarding-' + [guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($root) | Out-Null
$names = @('CODEX_HOME', 'CLAUDE_CONFIG_DIR', 'LOCALAPPDATA', 'PATH', 'TEST_CLAUDE_LOG', 'CLAUDE_CODE_SIMPLE', 'CLAUDE_CODE_DISABLE_PROJECT_INSTRUCTIONS')
$saved = @{}; foreach ($name in $names) { $saved[$name] = [Environment]::GetEnvironmentVariable($name) }
function Fail([string]$m) { throw "hooks onboarding: $m" }
try {
    $env:CLAUDE_CODE_SIMPLE = $null; $env:CLAUDE_CODE_DISABLE_PROJECT_INSTRUCTIONS = $null
    $env:CLAUDE_CONFIG_DIR = Join-Path $root '.claude'
    $env:CODEX_HOME = Join-Path $root '.codex'
    $env:LOCALAPPDATA = Join-Path $root 'local'
    $env:TEST_CLAUDE_LOG = Join-Path $root 'claude-calls'
    $settings = Join-Path $root '.claude/settings.json'
    $helperCopy = Join-Path $root 'local/Programs/mmhmem/manage_hooks.ps1'
    # Disposable engine copy whose $HOME is the scratch root and which never edits the user PATH.
    $tree = Join-Path $root 'engine'; [IO.Directory]::CreateDirectory($tree) | Out-Null
    Copy-Item (Join-Path $PSScriptRoot '../deploy') $tree -Recurse
    $engine = Join-Path $tree 'deploy/install_local.ps1'
    $source = [IO.File]::ReadAllText($engine)
    $pathWrite = "[Environment]::SetEnvironmentVariable('Path', ((`$entries + `$UserBinDir) -join ';'), 'User')"
    if (-not $source.Contains($pathWrite)) { Fail 'engine PATH write not found; update this test' }
    $source = $source.Replace($pathWrite, '$null').Replace('$HOME', '$HooksTestRoot')
    $source = $source.Replace('$ErrorActionPreference = ''Stop''', ('$ErrorActionPreference = ''Stop''' + "`n" + '$HooksTestRoot = ''' + $root.Replace("'", "''") + ''''))
    [IO.File]::WriteAllText($engine, $source, (New-Object Text.UTF8Encoding($false)))
    $fake = Join-Path $root 'mmhmem.exe'; [IO.File]::WriteAllText($fake, 'fixture')
    [IO.File]::WriteAllText((Join-Path $root 'claude.cmd'), '@echo called>>"%TEST_CLAUDE_LOG%"' + "`r`n" + '@exit /b 0')
    $env:PATH = $root + ';' + $saved['PATH']
    $shell = (Get-Process -Id $PID).Path
    function Invoke-Engine([string[]]$EngineArgs, [string]$AgentName = 'claude') {
        $old = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
        try {
            $text = & $shell -NoProfile -File $engine -Global -Binary $fake -Agent $AgentName -NonInteractive @EngineArgs 2>&1 | ForEach-Object { [string]$_ } | Out-String
            return [pscustomobject]@{ Code = $LASTEXITCODE; Text = $text }
        } finally { $ErrorActionPreference = $old }
    }
    $original = "{`n  `"model`": `"opus`",`n  `"hooks`": {`n    `"PreToolUse`": [`n      {`n        `"matcher`": `"Bash`",`n        `"hooks`": [ { `"type`": `"command`", `"command`": `"other-plugin pre`" } ]`n      }`n    ]`n  },`n  `"ratio`": 1.50`n}`n"
    [IO.Directory]::CreateDirectory((Join-Path $root '.claude')) | Out-Null
    $utf8 = New-Object Text.UTF8Encoding($false)
    [IO.File]::WriteAllText($settings, $original, $utf8)
    function Assert-Settings([string]$Expected, [string]$What) {
        if ([IO.File]::ReadAllText($settings) -cne $Expected) { Fail "settings changed: $What" }
    }
    function Count-Entries { ([regex]::Matches([IO.File]::ReadAllText($settings), 'mmhmem\.exe')).Count }

    # Default install adds no hook and says how to opt in.
    $r = Invoke-Engine @()
    if ($r.Code -ne 0) { Fail "default install failed: $($r.Text)" }
    Assert-Settings $original 'default install'
    if (Test-Path -LiteralPath $helperCopy) { Fail 'default install copied the hook helper' }
    if ($r.Text -notmatch '-Hooks claude') { Fail "default install did not mention the opt-in: $($r.Text)" }

    # Dry runs write nothing but still show the entry.
    $r = Invoke-Engine @('-Hooks', 'claude', '-DryRun')
    if ($r.Code -ne 0) { Fail "opt-in dry run failed: $($r.Text)" }
    Assert-Settings $original 'dry run'
    if (Test-Path -LiteralPath $helperCopy) { Fail 'dry run copied the helper' }
    if ($r.Text -notmatch 'Remove it with' -or $r.Text -notmatch 'What it sees and decides' -or $r.Text -notmatch [regex]::Escape($settings)) { Fail "dry run did not show the entry: $($r.Text)" }
    if ($r.Text -notmatch [regex]::Escape($helperCopy) -or $r.Text -notmatch '-RemoveHooks' -or $r.Text -match 'engine[\\/]deploy[\\/]manage_hooks\.ps1') { Fail "dry run names a removal that will not exist after install: $($r.Text)" }

    # Opt-in adds exactly one entry; repeating keeps one.
    $r = Invoke-Engine @('-Hooks', 'claude')
    if ($r.Code -ne 0) { Fail "opt-in failed: $($r.Text)" }
    if ((Count-Entries) -ne 1) { Fail 'opt-in did not add exactly one entry' }
    if (-not (Test-Path -LiteralPath $helperCopy)) { Fail 'persistent helper missing' }
    if ($r.Text -notmatch '/hooks' -or $r.Text -notmatch 'advisory guidance only' -or $r.Text -notmatch 'Remove it with') { Fail "missing approval or advisory text: $($r.Text)" }
    if ($r.Text -notmatch [regex]::Escape($helperCopy)) { Fail 'removal command does not name the persistent helper' }
    $r = Invoke-Engine @('-Hooks', 'claude')
    if ($r.Code -ne 0 -or (Count-Entries) -ne 1) { Fail "repeat opt-in: $($r.Text)" }
    $installed = [IO.File]::ReadAllText($settings)

    # Dry-run removals leave the entry; a default re-install does not drop it.
    $null = Invoke-Engine @('-RemoveHooks', '-DryRun'); Assert-Settings $installed 'RemoveHooks dry run'
    $null = Invoke-Engine @('-Remove', '-DryRun'); Assert-Settings $installed 'Remove dry run'
    $null = Invoke-Engine @(); Assert-Settings $installed 'default re-install'

    # Hook-only removal restores the settings bytes and leaves the rest of the install.
    $r = Invoke-Engine @('-RemoveHooks')
    if ($r.Code -ne 0) { Fail "RemoveHooks failed: $($r.Text)" }
    Assert-Settings $original 'RemoveHooks'
    if (Test-Path -LiteralPath $helperCopy) { Fail 'RemoveHooks left the persistent helper' }
    if (-not (Test-Path -LiteralPath (Join-Path $root 'local/Programs/mmhmem/mmhmem.exe'))) { Fail 'RemoveHooks removed the binary' }
    if (-not (Test-Path -LiteralPath (Join-Path $root '.claude/rules/mmhmem.md'))) { Fail 'RemoveHooks removed the routing rule' }

    # Full removal removes the entry as well.
    $null = Invoke-Engine @('-Hooks', 'claude')
    if ((Count-Entries) -ne 1) { Fail 'second opt-in missing' }
    $r = Invoke-Engine @('-Remove')
    if ($r.Code -ne 0) { Fail "full removal failed: $($r.Text)" }
    Assert-Settings $original 'full removal'

    # Codex consent uses its own user hooks file and leaves approval to /hooks.
    [IO.Directory]::CreateDirectory($env:CODEX_HOME) | Out-Null
    $codexHooks = Join-Path $env:CODEX_HOME 'hooks.json'
    $r = Invoke-Engine @() -AgentName codex
    if ($r.Code -ne 0 -or (Test-Path $codexHooks)) { Fail "Codex default install: $($r.Text)" }
    $r = Invoke-Engine @('-Hooks', 'codex', '-DryRun') -AgentName codex
    if ($r.Code -ne 0 -or (Test-Path $codexHooks) -or $r.Text -notmatch '-Harness codex') { Fail "Codex dry run: $($r.Text)" }
    $r = Invoke-Engine @('-Hooks', 'codex') -AgentName codex
    if ($r.Code -ne 0 -or -not (Test-Path $codexHooks) -or $r.Text -notmatch '/hooks') { Fail "Codex opt-in: $($r.Text)" }
    $codexInstalled = [IO.File]::ReadAllText($codexHooks)
    $r = Invoke-Engine @('-Hooks', 'codex') -AgentName codex
    if ($r.Code -ne 0 -or [IO.File]::ReadAllText($codexHooks) -cne $codexInstalled) { Fail "Codex repeat: $($r.Text)" }
    $r = Invoke-Engine @('-RemoveHooks') -AgentName codex
    if ($r.Code -ne 0 -or [IO.File]::ReadAllText($codexHooks) -match 'hook codex') { Fail "Codex hook removal: $($r.Text)" }

    # Refusals happen before any write.
    foreach ($bad in @(@('-Hooks', 'codex'), @('-Hooks', 'bogus'), @('-Hooks', 'claude', '-RemoveHooks'))) {
        $r = Invoke-Engine $bad
        if ($r.Code -eq 0) { Fail "accepted: $($bad -join ' ')" }
        Assert-Settings $original ("refused " + ($bad -join ' '))
    }
    $r = Invoke-Engine @('-Remove', '-RemoveHooks')
    if ($r.Code -eq 0 -or $r.Text -notmatch 'already removes') { Fail "-Remove -RemoveHooks gave no clear message: $($r.Text)" }
    $r = & $shell -NoProfile -File $engine -ProjectDir $root -Binary $fake -Agent claude -NonInteractive -Hooks claude 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) { Fail 'project-scope opt-in accepted' }
    Assert-Settings $original 'project-scope opt-in'
    Remove-Item -LiteralPath $env:TEST_CLAUDE_LOG -ErrorAction SilentlyContinue
    $blocked = $original.Replace('"model"', '"disableAllHooks": true, "model"')
    [IO.File]::WriteAllText($settings, $blocked, $utf8)
    $r = Invoke-Engine @('-Hooks', 'claude')
    if ($r.Code -eq 0 -or $r.Text -notmatch 'disableAllHooks' -or $r.Text -notmatch 'advisory guidance only') { Fail "policy refusal missing: $($r.Text)" }
    Assert-Settings $blocked 'policy refusal'
    if (Test-Path -LiteralPath $env:TEST_CLAUDE_LOG) { Fail 'policy refusal happened after harness registration' }
    Write-Host 'PASS: hook opt-in default off, dry run, one entry, repeat, hook-only and full removal, refusals before writes'
} finally {
    foreach ($name in $names) { [Environment]::SetEnvironmentVariable($name, $saved[$name]) }
    if (Test-Path -LiteralPath $root) { Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue }
}
