# Offline checks for deploy/manage_hooks.ps1 (Claude Code entry, Windows).
# Every path is a temporary override: no real harness settings are read or written.
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$helper = Join-Path $repo 'deploy/manage_hooks.ps1'
$engine = (Get-Process -Id $PID).Path
$scratch = Join-Path $repo ('tmp/manage-hooks-tests-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $scratch | Out-Null
$utf8 = New-Object Text.UTF8Encoding($false)
$failures = New-Object Collections.Generic.List[string]

function Invoke-Helper {
    param([string[]]$Arguments, [string]$WorkDir = $scratch)
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    Push-Location -LiteralPath $WorkDir
    try {
        $out = & $engine -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $helper @Arguments 2>&1 | Out-String
        $code = $LASTEXITCODE
    } finally { Pop-Location; $ErrorActionPreference = $previous }
    return @{ Code = $code; Out = $out }
}
function Check([bool]$Condition, [string]$Name, [string]$Detail = '') {
    if ($Condition) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name $Detail"; $failures.Add($Name) }
}
function Write-Text([string]$Path, [string]$Text) {
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Path) | Out-Null
    [IO.File]::WriteAllBytes($Path, $utf8.GetBytes($Text))
}
function Get-Bytes([string]$Path) { [Convert]::ToBase64String([IO.File]::ReadAllBytes($Path)) }
function Read-Json([string]$Path) { [IO.File]::ReadAllText($Path) | ConvertFrom-Json }
function Get-Ours($Json) {
    $found = @()
    if ($Json.hooks -and $Json.hooks.PreToolUse) {
        foreach ($group in @($Json.hooks.PreToolUse)) {
            foreach ($handler in @($group.hooks)) {
                $name = [IO.Path]::GetFileName([string]$handler.command)
                if (($name -eq 'mmhmem.exe' -or $name -eq 'mmhmem') -and (@($handler.args) -join '|') -eq 'hook|claude') { $found += ,@($group, $handler) }
            }
        }
    }
    return ,$found
}
function Case([string]$Name) { $dir = Join-Path $scratch $Name; New-Item -ItemType Directory -Force -Path $dir | Out-Null; return $dir }

$binDir = Join-Path $scratch 'bin dir'
New-Item -ItemType Directory -Force -Path $binDir | Out-Null
$bin = Join-Path $binDir 'mmhmem.exe'
[IO.File]::WriteAllText($bin, 'fixture')
$bin2Dir = Join-Path $scratch 'other'
New-Item -ItemType Directory -Force -Path $bin2Dir | Out-Null
$bin2 = Join-Path $bin2Dir 'mmhmem.exe'
[IO.File]::WriteAllText($bin2, 'fixture')
$noManaged = Join-Path $scratch 'no-managed/managed-settings.json'

$neighbour = @'
{
  "$schema": "https://json.schemastore.org/claude-code-settings.json",
  "model": "opus",
  "permissions": {
    "allow": [
      "Bash(npm run lint)"
    ],
    "deny": []
  },
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "other-memory-plugin pre",
            "timeout": 5
          }
        ]
      }
    ],
    "PostToolUse": [
      {
        "matcher": "Edit",
        "hooks": [
          {
            "type": "command",
            "command": "fmt"
          }
        ]
      }
    ]
  },
  "note": "it\u0027s caf{e} \"quoted\"",
  "ratio": 1.50
}
'@ -replace "`r`n", "`n"
$neighbour = $neighbour.Replace('{e}', [string][char]0xE9)

try {
    # 1. install into a missing file and directory
    $dir = Case 'missing'
    $settings = Join-Path $dir 'deep/claude-home/settings.json'
    $r = Invoke-Helper @('install', '-Harness', 'claude', '-Binary', $bin, '-SettingsPath', $settings, '-ManagedSettingsPath', $noManaged)
    Check ($r.Code -eq 0) 'install into missing file succeeds' $r.Out
    Check (Test-Path -LiteralPath $settings) 'install creates the settings file'
    if (Test-Path -LiteralPath $settings) {
        $raw = [IO.File]::ReadAllBytes($settings)
        Check (-not ($raw.Length -ge 3 -and $raw[0] -eq 0xEF -and $raw[1] -eq 0xBB -and $raw[2] -eq 0xBF)) 'created file has no BOM'
        $json = Read-Json $settings
        $ours = Get-Ours $json
        Check ($ours.Count -eq 1) 'created file holds one mmhmem entry'
        if ($ours.Count -eq 1) {
            Check ($ours[0][0].matcher -eq 'Read|Bash|PowerShell') 'matcher covers Read, Bash and PowerShell'
            Check ($ours[0][1].command -eq $bin -and $ours[0][1].type -eq 'command' -and $ours[0][1].timeout -eq 10) 'exec form with absolute binary and timeout 10'
            Check ($null -eq $ours[0][1].PSObject.Properties['shell']) 'entry sets no shell'
        }
    }
    Check (-not (Test-Path -LiteralPath ($settings + '.mmhmem.bak'))) 'no backup for a file that did not exist'

    # 2. install next to other settings and an unrelated PreToolUse hook
    $dir = Case 'neighbours'
    $settings = Join-Path $dir 'settings.json'
    Write-Text $settings $neighbour
    $original = Get-Bytes $settings
    $r = Invoke-Helper @('install', '-Harness', 'claude', '-Binary', $bin, '-SettingsPath', $settings, '-ManagedSettingsPath', $noManaged)
    Check ($r.Code -eq 0) 'install next to neighbours succeeds' $r.Out
    Check ((Test-Path -LiteralPath ($settings + '.mmhmem.bak')) -and (Get-Bytes ($settings + '.mmhmem.bak')) -eq $original) 'backup holds the original bytes'
    $json = Read-Json $settings
    $before = $neighbour | ConvertFrom-Json
    Check ((($json.PSObject.Properties.Name) -join ',') -eq (($before.PSObject.Properties.Name) -join ',')) 'top-level key order preserved'
    Check ((($json.hooks.PSObject.Properties.Name) -join ',') -eq 'PreToolUse,PostToolUse') 'hook events kept in order'
    Check (@($json.hooks.PreToolUse).Count -eq 2 -and @($json.hooks.PreToolUse)[0].hooks[0].command -eq 'other-memory-plugin pre') 'unrelated PreToolUse group kept first'
    Check ((Get-Ours $json).Count -eq 1 -and @($json.hooks.PreToolUse)[1].hooks[0].command -eq $bin) 'mmhmem group appended after it'
    Check (@($json.permissions.allow).Count -eq 1 -and @($json.permissions.deny).Count -eq 0) 'single-element and empty arrays survive'
    $text = [IO.File]::ReadAllText($settings)
    Check ($text.Contains(('"note": "it\u0027s caf{e} \"quoted\""').Replace('{e}', [string][char]0xE9)) -and $text.Contains('"ratio": 1.50')) 'untouched values keep their exact text'

    # 3. repeated install keeps exactly one entry
    $afterFirst = Get-Bytes $settings
    $r = Invoke-Helper @('install', '-Harness', 'claude', '-Binary', $bin, '-SettingsPath', $settings, '-ManagedSettingsPath', $noManaged)
    Check ($r.Code -eq 0 -and (Get-Bytes $settings) -eq $afterFirst) 'repeated install leaves the file unchanged' $r.Out
    Check ((Get-Ours (Read-Json $settings)).Count -eq 1) 'repeated install keeps one entry'

    # 4. a different binary path replaces the entry in place
    $r = Invoke-Helper @('install', '-Harness', 'claude', '-Binary', $bin2, '-SettingsPath', $settings, '-ManagedSettingsPath', $noManaged)
    $json = Read-Json $settings
    Check ($r.Code -eq 0 -and $r.Out -match '(?i)replac') 'binary change is reported as a replacement' $r.Out
    Check ((Get-Ours $json).Count -eq 1 -and @($json.hooks.PreToolUse)[1].hooks[0].command -eq $bin2) 'entry replaced in place'

    # 5. remove restores the original exactly
    $r = Invoke-Helper @('remove', '-Harness', 'claude', '-SettingsPath', $settings, '-ManagedSettingsPath', $noManaged)
    Check ($r.Code -eq 0) 'remove succeeds' $r.Out
    Check ((Get-Bytes $settings) -eq $original) 'remove restores the original bytes'
    $json = Read-Json $settings
    Check ((($json.PSObject.Properties.Name) -join ',') -eq (($before.PSObject.Properties.Name) -join ',') -and ($json | ConvertTo-Json -Depth 20) -eq ($before | ConvertTo-Json -Depth 20)) 'remove restores parsed content and key order'

    # 6. remove when absent is a no-op
    $stamp = (Get-Item -LiteralPath $settings).LastWriteTimeUtc
    Remove-Item -LiteralPath ($settings + '.mmhmem.bak') -ErrorAction SilentlyContinue
    $r = Invoke-Helper @('remove', '-Harness', 'claude', '-SettingsPath', $settings, '-ManagedSettingsPath', $noManaged)
    Check ($r.Code -eq 0 -and (Get-Bytes $settings) -eq $original -and (Get-Item -LiteralPath $settings).LastWriteTimeUtc -eq $stamp) 'remove when absent writes nothing' $r.Out
    Check (-not (Test-Path -LiteralPath ($settings + '.mmhmem.bak'))) 'remove when absent makes no backup'
    $missing = Join-Path $dir 'none/settings.json'
    $r = Invoke-Helper @('remove', '-Harness', 'claude', '-SettingsPath', $missing, '-ManagedSettingsPath', $noManaged)
    Check ($r.Code -eq 0 -and -not (Test-Path -LiteralPath $missing)) 'remove with no settings file is a no-op' $r.Out

    # 7. other formatting (4 spaces, CRLF, no hooks) round-trips byte for byte
    $dir = Case 'crlf'
    $settings = Join-Path $dir 'settings.json'
    Write-Text $settings ("{`r`n    `"theme`": `"dark`",`r`n    `"env`": {`r`n        `"A`": `"1`"`r`n    }`r`n}`r`n")
    $original = Get-Bytes $settings
    $r = Invoke-Helper @('install', '-Harness', 'claude', '-Binary', $bin, '-SettingsPath', $settings, '-ManagedSettingsPath', $noManaged)
    $text = [IO.File]::ReadAllText($settings)
    Check ($r.Code -eq 0 -and (Get-Ours (Read-Json $settings)).Count -eq 1) 'install into a file without hooks' $r.Out
    Check (-not ($text -match "[^`r]`n")) 'CRLF line endings kept'
    Check ($text.Contains("`r`n    `"hooks`": {`r`n        `"PreToolUse`": [")) 'new keys follow the file indentation'
    $r = Invoke-Helper @('remove', '-Harness', 'claude', '-SettingsPath', $settings, '-ManagedSettingsPath', $noManaged)
    Check ($r.Code -eq 0 -and (Get-Bytes $settings) -eq $original) 'remove restores a CRLF file byte for byte' $r.Out

    # 8. a shared group: remove deletes only the mmhmem handler
    $dir = Case 'shared-group'
    $settings = Join-Path $dir 'settings.json'
    $shared = '{"hooks":{"PreToolUse":[{"matcher":"Read","hooks":[{"type":"command","command":"guard"},{"type":"command","command":"C:\\old\\mmhmem.exe","args":["hook","claude"],"timeout":10}]}]}}'
    Write-Text $settings $shared
    $r = Invoke-Helper @('install', '-Harness', 'claude', '-Binary', $bin, '-SettingsPath', $settings, '-ManagedSettingsPath', $noManaged)
    $json = Read-Json $settings
    Check ($r.Code -eq 0 -and @($json.hooks.PreToolUse).Count -eq 1 -and @($json.hooks.PreToolUse)[0].hooks[1].command -eq $bin) 'existing entry in a shared group updated, no second group' $r.Out
    $r = Invoke-Helper @('remove', '-Harness', 'claude', '-SettingsPath', $settings, '-ManagedSettingsPath', $noManaged)
    Check ($r.Code -eq 0 -and [IO.File]::ReadAllText($settings) -eq '{"hooks":{"PreToolUse":[{"matcher":"Read","hooks":[{"type":"command","command":"guard"}]}]}}') 'remove keeps the shared group and its other handler' ([IO.File]::ReadAllText($settings))

    # 9. a different args list or file name is not mmhmem's entry
    $dir = Case 'identity'
    $settings = Join-Path $dir 'settings.json'
    $foreign = '{"hooks":{"PreToolUse":[{"hooks":[{"type":"command","command":"C:\\x\\mmhmem.exe","args":["hook","codex"]},{"type":"command","command":"C:\\x\\notmmhmem.exe","args":["hook","claude"]}]}]}}'
    Write-Text $settings $foreign
    $r = Invoke-Helper @('remove', '-Harness', 'claude', '-SettingsPath', $settings, '-ManagedSettingsPath', $noManaged)
    Check ($r.Code -eq 0 -and [IO.File]::ReadAllText($settings) -eq $foreign) 'remove ignores handlers that are not identified' $r.Out

    # 9b. compact files stay compact, an empty hooks object round-trips, duplicates collapse to one
    $dir = Case 'compact'
    $settings = Join-Path $dir 'settings.json'
    Write-Text $settings '{"hooks":{}}'
    $r = Invoke-Helper @('install', '-Harness', 'claude', '-Binary', $bin, '-SettingsPath', $settings, '-ManagedSettingsPath', $noManaged)
    $text = [IO.File]::ReadAllText($settings)
    Check ($r.Code -eq 0 -and -not $text.Contains("`n") -and (Get-Ours (Read-Json $settings)).Count -eq 1) 'compact file stays on one line' $text
    $r = Invoke-Helper @('remove', '-Harness', 'claude', '-SettingsPath', $settings, '-ManagedSettingsPath', $noManaged)
    Check ($r.Code -eq 0 -and [IO.File]::ReadAllText($settings) -eq '{}') 'emptied hooks object dropped on remove' ([IO.File]::ReadAllText($settings))
    $handler = '{"type":"command","command":"C:\\a\\mmhmem.exe","args":["hook","claude"],"timeout":10}'
    Write-Text $settings ('{"hooks":{"PreToolUse":[{"matcher":"Read","hooks":[' + $handler + ']},{"matcher":"Bash","hooks":[' + $handler + ']}]}}')
    $r = Invoke-Helper @('install', '-Harness', 'claude', '-Binary', $bin, '-SettingsPath', $settings, '-ManagedSettingsPath', $noManaged)
    $ours = Get-Ours (Read-Json $settings)
    Check ($r.Code -eq 0 -and $ours.Count -eq 1 -and $ours[0][1].command -eq $bin) 'install collapses duplicate entries to one' $r.Out
    $r = Invoke-Helper @('remove', '-Harness', 'claude', '-SettingsPath', $settings, '-ManagedSettingsPath', $noManaged)
    Check ($r.Code -eq 0 -and [IO.File]::ReadAllText($settings) -eq '{}') 'remove of the last entry drops the emptied containers' ([IO.File]::ReadAllText($settings))

    # 10. unparsable settings are refused unchanged
    $dir = Case 'broken'
    $settings = Join-Path $dir 'settings.json'
    Write-Text $settings '{"model": "opus",, }'
    $original = Get-Bytes $settings
    foreach ($op in @('install', 'remove')) {
        $r = Invoke-Helper @($op, '-Harness', 'claude', '-Binary', $bin, '-SettingsPath', $settings, '-ManagedSettingsPath', $noManaged)
        Check ($r.Code -ne 0 -and (Get-Bytes $settings) -eq $original -and -not (Test-Path -LiteralPath ($settings + '.mmhmem.bak'))) "$op refuses unparsable settings unchanged" $r.Out
    }

    # 11. policy refusals name the setting
    $dir = Case 'policy-user'
    $settings = Join-Path $dir 'settings.json'
    Write-Text $settings "{`n  `"disableAllHooks`": true`n}`n"
    $original = Get-Bytes $settings
    $r = Invoke-Helper @('install', '-Harness', 'claude', '-Binary', $bin, '-SettingsPath', $settings, '-ManagedSettingsPath', $noManaged)
    Check ($r.Code -ne 0 -and $r.Out -match 'disableAllHooks' -and (Get-Bytes $settings) -eq $original) 'user disableAllHooks refuses install' $r.Out
    $r = Invoke-Helper @('status', '-Harness', 'claude', '-SettingsPath', $settings, '-ManagedSettingsPath', $noManaged)
    Check ($r.Out -match 'policy-blocked' -and $r.Out -match 'disableAllHooks') 'status reports the user policy block' $r.Out
    $managedCases = [ordered]@{
        'disableAllHooks' = '{"disableAllHooks": true}'
        'allowManagedHooksOnly' = '{"allowManagedHooksOnly": true}'
        'strictPluginOnlyCustomization' = '{"strictPluginOnlyCustomization": ["skills", "hooks"]}'
    }
    foreach ($key in $managedCases.Keys) {
        $dir = Case ('policy-managed-' + $key)
        $settings = Join-Path $dir 'settings.json'
        $managed = Join-Path $dir 'ClaudeCode/managed-settings.json'
        Write-Text $managed $managedCases[$key]
        $r = Invoke-Helper @('install', '-Harness', 'claude', '-Binary', $bin, '-SettingsPath', $settings, '-ManagedSettingsPath', $managed)
        Check ($r.Code -ne 0 -and $r.Out -match $key -and $r.Out -match 'managed' -and -not (Test-Path -LiteralPath $settings)) "managed $key refuses install" $r.Out
    }
    $dir = Case 'policy-dropin'
    $settings = Join-Path $dir 'settings.json'
    $managed = Join-Path $dir 'ClaudeCode/managed-settings.json'
    Write-Text (Join-Path $dir 'ClaudeCode/managed-settings.d/10-security.json') '{"disableAllHooks": true}'
    $r = Invoke-Helper @('install', '-Harness', 'claude', '-Binary', $bin, '-SettingsPath', $settings, '-ManagedSettingsPath', $managed)
    Check ($r.Code -ne 0 -and $r.Out -match 'disableAllHooks' -and $r.Out -match '10-security' -and -not (Test-Path -LiteralPath $settings)) 'managed drop-in disableAllHooks refuses install' $r.Out
    $dir = Case 'policy-off'
    $settings = Join-Path $dir 'settings.json'
    $managed = Join-Path $dir 'ClaudeCode/managed-settings.json'
    Write-Text $managed '{"disableAllHooks": false, "strictPluginOnlyCustomization": ["skills"]}'
    $r = Invoke-Helper @('install', '-Harness', 'claude', '-Binary', $bin, '-SettingsPath', $settings, '-ManagedSettingsPath', $managed)
    Check ($r.Code -eq 0 -and (Get-Ours (Read-Json $settings)).Count -eq 1) 'non-blocking managed values allow install' $r.Out
    $r = Invoke-Helper @('status', '-Harness', 'claude', '-SettingsPath', $settings, '-ManagedSettingsPath', $managed)
    Check ($r.Code -eq 0 -and $r.Out -match '(?m)\binstalled\b' -and $r.Out -notmatch 'not installed') 'status reports installed' $r.Out
    $r = Invoke-Helper @('status', '-Harness', 'claude', '-SettingsPath', (Join-Path $dir 'none.json'), '-ManagedSettingsPath', $managed)
    Check ($r.Code -eq 0 -and $r.Out -match 'not installed') 'status reports not installed' $r.Out

    # 12. show and dry runs write nothing
    $dir = Case 'show'
    $settings = Join-Path $dir 'cfg/settings.json'
    $r = Invoke-Helper @('show', '-Harness', 'claude', '-Binary', $bin, '-SettingsPath', $settings, '-ManagedSettingsPath', $noManaged)
    $quoted = $bin.Replace('\', '\\')
    Check ($r.Code -eq 0 -and $r.Out.Contains($quoted) -and $r.Out.Contains('"hook"') -and $r.Out.Contains($settings) -and $r.Out -match 'remove' -and $r.Out -match 'smart_read') 'show prints entry, file, behaviour and removal' $r.Out
    Check (-not (Test-Path -LiteralPath (Join-Path $dir 'cfg'))) 'show writes nothing'
    $r = Invoke-Helper @('install', '-Harness', 'claude', '-Binary', $bin, '-SettingsPath', $settings, '-ManagedSettingsPath', $noManaged, '-DryRun')
    Check ($r.Code -eq 0 -and $r.Out -match 'dry-run' -and -not (Test-Path -LiteralPath (Join-Path $dir 'cfg'))) 'dry-run install writes nothing' $r.Out
    $settings = Join-Path $dir 'settings.json'
    Write-Text $settings $neighbour
    Invoke-Helper @('install', '-Harness', 'claude', '-Binary', $bin, '-SettingsPath', $settings, '-ManagedSettingsPath', $noManaged) | Out-Null
    Remove-Item -LiteralPath ($settings + '.mmhmem.bak') -ErrorAction SilentlyContinue
    $installed = Get-Bytes $settings
    $r = Invoke-Helper @('remove', '-Harness', 'claude', '-SettingsPath', $settings, '-ManagedSettingsPath', $noManaged, '-DryRun')
    Check ($r.Code -eq 0 -and (Get-Bytes $settings) -eq $installed -and -not (Test-Path -LiteralPath ($settings + '.mmhmem.bak'))) 'dry-run remove writes nothing' $r.Out
    $r = Invoke-Helper @('install', '-Harness', 'claude', '-Binary', $bin2, '-SettingsPath', $settings, '-ManagedSettingsPath', $noManaged, '-DryRun')
    Check ($r.Code -eq 0 -and (Get-Bytes $settings) -eq $installed) 'dry-run replacement writes nothing' $r.Out

    # 13. CLAUDE_CONFIG_DIR selects the user settings file
    $configDir = Join-Path (Case 'config-dir') 'claude-home'
    $savedConfig = $env:CLAUDE_CONFIG_DIR
    $env:CLAUDE_CONFIG_DIR = $configDir
    try { $r = Invoke-Helper @('show', '-Harness', 'claude', '-Binary', $bin, '-ManagedSettingsPath', $noManaged) }
    finally { $env:CLAUDE_CONFIG_DIR = $savedConfig }
    Check ($r.Code -eq 0 -and $r.Out.Contains((Join-Path $configDir 'settings.json'))) 'CLAUDE_CONFIG_DIR selects the target file' $r.Out

    # 14. user scope only: a project's settings files are never touched
    $project = Case 'project'
    Write-Text (Join-Path $project '.claude/settings.json') '{"model":"sonnet"}'
    Write-Text (Join-Path $project '.claude/settings.local.json') '{}'
    $projectState = (Get-ChildItem -LiteralPath (Join-Path $project '.claude') -Force | ForEach-Object { $_.Name + '=' + (Get-Bytes $_.FullName) }) -join ';'
    $settings = Join-Path (Case 'project-user') 'settings.json'
    $r = Invoke-Helper @('install', '-Harness', 'claude', '-Binary', $bin, '-SettingsPath', $settings, '-ManagedSettingsPath', $noManaged) -WorkDir $project
    $after = (Get-ChildItem -LiteralPath (Join-Path $project '.claude') -Force | ForEach-Object { $_.Name + '=' + (Get-Bytes $_.FullName) }) -join ';'
    Check ($r.Code -eq 0 -and $after -eq $projectState -and @(Get-ChildItem -LiteralPath $project -Force).Count -eq 1) 'project settings untouched' $r.Out
    foreach ($bad in @((Join-Path $project '.claude/settings.json'), (Join-Path $project '.claude/settings.local.json'))) {
        $r = Invoke-Helper @('install', '-Harness', 'claude', '-Binary', $bin, '-SettingsPath', $bad, '-ManagedSettingsPath', $noManaged) -WorkDir $project
        Check ($r.Code -ne 0) "refuses a project settings target $([IO.Path]::GetFileName($bad))" $r.Out
    }
    $after = (Get-ChildItem -LiteralPath (Join-Path $project '.claude') -Force | ForEach-Object { $_.Name + '=' + (Get-Bytes $_.FullName) }) -join ';'
    Check ($after -eq $projectState) 'project settings still untouched after refused targets'

    # 15. the binary must be an absolute mmhmem path
    $r = Invoke-Helper @('install', '-Harness', 'claude', '-Binary', 'mmhmem.exe', '-SettingsPath', (Join-Path $scratch 'rel.json'), '-ManagedSettingsPath', $noManaged)
    Check ($r.Code -ne 0 -and -not (Test-Path -LiteralPath (Join-Path $scratch 'rel.json'))) 'relative binary refused'
    $r = Invoke-Helper @('install', '-Harness', 'claude', '-Binary', '\x\mmhmem.exe', '-SettingsPath', (Join-Path $scratch 'drel.json'), '-ManagedSettingsPath', $noManaged)
    Check ($r.Code -ne 0 -and -not (Test-Path -LiteralPath (Join-Path $scratch 'drel.json'))) 'drive-relative binary refused'

    # 16. JSON keys are case-sensitive: "Hooks" beside "hooks" is valid and kept
    $dir = Case 'case-keys'; $settings = Join-Path $dir 'settings.json'
    Write-Text $settings "{`n  `"Hooks`": 1,`n  `"hooks`": {}`n}`n"
    $r = Invoke-Helper @('install', '-Harness', 'claude', '-Binary', $bin, '-SettingsPath', $settings, '-ManagedSettingsPath', $noManaged)
    Check ($r.Code -eq 0 -and ([IO.File]::ReadAllText($settings)).Contains('"Hooks": 1')) 'case-distinct keys are not duplicates' $r.Out
} finally {
    Remove-Item -LiteralPath $scratch -Recurse -Force -ErrorAction SilentlyContinue
}
if ($failures.Count) { throw "manage_hooks tests failed ($($failures.Count)): $($failures -join '; ')" }
Write-Host "PASS: manage_hooks Claude Code entry ($($PSVersionTable.PSEdition) $($PSVersionTable.PSVersion))"
