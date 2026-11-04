# Offline checks for deploy/manage_hooks.ps1 -Harness codex (read-routing-hooks task 4.1).
# Every path is a temporary override: no real Codex home is read or written.
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$helper = Join-Path $repo 'deploy/manage_hooks.ps1'
$engine = (Get-Process -Id $PID).Path
$scratch = Join-Path $repo ('tmp/manage-hooks-codex-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $scratch | Out-Null
$utf8 = New-Object Text.UTF8Encoding($false)
$failures = New-Object Collections.Generic.List[string]

function Invoke-Helper {
    param([string[]]$Arguments, [string]$WorkDir = $scratch, [string]$CodexHome)
    $previous = $ErrorActionPreference; $savedHome = $env:CODEX_HOME
    $ErrorActionPreference = 'Continue'
    if ($CodexHome) { $env:CODEX_HOME = $CodexHome }
    Push-Location -LiteralPath $WorkDir
    try {
        $out = & $engine -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $helper @Arguments 2>&1 | ForEach-Object { [string]$_ } | Out-String
        $code = $LASTEXITCODE
    } finally { Pop-Location; $ErrorActionPreference = $previous; $env:CODEX_HOME = $savedHome }
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
function Get-Ours([string]$Path) {
    $json = [IO.File]::ReadAllText($Path) | ConvertFrom-Json
    $found = @()
    foreach ($group in @($json.hooks.PreToolUse)) {
        foreach ($handler in @($group.hooks)) { if ([string]$handler.command -like '*mmhmem.exe'' hook codex') { $found += , @($group, $handler) } }
    }
    return , $found
}
function Case([string]$Name) { $dir = Join-Path $scratch $Name; New-Item -ItemType Directory -Force -Path $dir | Out-Null; return $dir }

$binDir = Join-Path $scratch 'bin dir'
New-Item -ItemType Directory -Force -Path $binDir | Out-Null
$bin = Join-Path $binDir 'mmhmem.exe'
[IO.File]::WriteAllText($bin, 'fixture')
$bin2 = Join-Path (Case 'other') 'mmhmem.exe'
[IO.File]::WriteAllText($bin2, 'fixture')
$noManaged = Join-Path $scratch 'no-managed/requirements.toml'
$expectedCommand = "& '$bin' hook codex"

# The user's real layout: claude-mem trust state inline in config.toml, an MCP server, comments.
$config = @'
model = "gpt-6.1-sol"

[features]
hooks = true  # hooks stay on

[hooks.state."claude-mem@claude-mem-local:hooks/codex-hooks.json:pre_tool_use:0:0"]
trusted_hash = "sha256:60636c3d"
enabled = true

[mcp_servers.mmhmem]
command = "C:\\mmhmem\\mmhmem.exe"
args = ["stdio"]
'@ -replace "`r`n", "`n"
$neighbour = @'
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "^Bash$",
        "hooks": [
          {
            "type": "command",
            "command": "other-memory-plugin pre",
            "timeout": 30
          }
        ]
      }
    ],
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "caf{e} stop"
          }
        ]
      }
    ]
  }
}
'@ -replace "`r`n", "`n"
$neighbour = $neighbour.Replace('{e}', [string][char]0xE9)

try {
    # 1. install into a fresh Codex home selected by CODEX_HOME
    $codexHome = Join-Path (Case 'fresh') 'codex home'
    $r = Invoke-Helper @('install', '-Harness', 'codex', '-Binary', $bin, '-ManagedSettingsPath', $noManaged) -CodexHome $codexHome
    $hooksFile = Join-Path $codexHome 'hooks.json'
    Check ($r.Code -eq 0 -and (Test-Path -LiteralPath $hooksFile)) 'install creates CODEX_HOME/hooks.json' $r.Out
    if (Test-Path -LiteralPath $hooksFile) {
        $ours = Get-Ours $hooksFile
        Check ($ours.Count -eq 1) 'one mmhmem entry'
        if ($ours.Count -eq 1) {
            Check ($ours[0][0].matcher -ceq '^(Bash|shell)$') 'matcher is the Codex shell tool'
            Check ($ours[0][1].command -ceq $expectedCommand -and $ours[0][1].type -eq 'command' -and $ours[0][1].timeout -eq 10) 'command calls the absolute binary with fixed arguments' $ours[0][1].command
            Check ($null -eq $ours[0][1].PSObject.Properties['args']) 'Codex entry has no args member'
        }
    }
    Check ($r.Out -match '/hooks' -and $r.Out -match 'never writes') 'install names the /hooks approval step and that trust is not written' $r.Out
    Check ((Test-Path -LiteralPath $codexHome) -and @(Get-ChildItem -LiteralPath $codexHome -Force).Count -eq 1) 'only hooks.json is written (no trust or config file)'

    # 2. neighbours kept, idempotent, exact removal, config.toml never written
    $dir = Case 'neighbours'; $hooksFile = Join-Path $dir 'hooks.json'; $configFile = Join-Path $dir 'config.toml'
    Write-Text $hooksFile $neighbour; Write-Text $configFile $config
    $original = Get-Bytes $hooksFile; $configBytes = Get-Bytes $configFile
    $r = Invoke-Helper @('install', '-Harness', 'codex', '-Binary', $bin, '-SettingsPath', $hooksFile, '-ManagedSettingsPath', $noManaged)
    Check ($r.Code -eq 0) 'install next to other hooks succeeds' $r.Out
    Check ($r.Out -notmatch 'both') 'inline [hooks.state] alone gives no coexistence note' $r.Out
    $json = [IO.File]::ReadAllText($hooksFile) | ConvertFrom-Json
    Check (@($json.hooks.PreToolUse).Count -eq 2 -and $json.hooks.PreToolUse[0].hooks[0].command -eq 'other-memory-plugin pre') 'existing PreToolUse group kept first'
    Check ($json.hooks.Stop[0].hooks[0].command -eq ('caf' + [char]0xE9 + ' stop')) 'other events kept'
    $afterFirst = Get-Bytes $hooksFile
    $r = Invoke-Helper @('install', '-Harness', 'codex', '-Binary', $bin, '-SettingsPath', $hooksFile, '-ManagedSettingsPath', $noManaged)
    Check ($r.Code -eq 0 -and (Get-Bytes $hooksFile) -eq $afterFirst -and $r.Out -match 'unchanged') 'repeated install is byte-identical (trust hash kept)' $r.Out
    Check ((Get-Bytes $configFile) -eq $configBytes) 'config.toml (trust state) never written'
    $r = Invoke-Helper @('status', '-Harness', 'codex', '-SettingsPath', $hooksFile, '-ManagedSettingsPath', $noManaged)
    Check ($r.Code -eq 0 -and $r.Out -match 'installed' -and $r.Out -match 'not yet trusted') 'status reports an untrusted entry' $r.Out
    $r = Invoke-Helper @('remove', '-Harness', 'codex', '-SettingsPath', $hooksFile)
    Check ($r.Code -eq 0 -and (Get-Bytes $hooksFile) -eq $original) 'remove restores the original bytes' $r.Out
    Check ((Get-Bytes $configFile) -eq $configBytes) 'config.toml untouched by removal'

    # 3. a changed binary path changes the entry and says re-approval is needed (Req 4.8)
    $r = Invoke-Helper @('install', '-Harness', 'codex', '-Binary', $bin, '-SettingsPath', $hooksFile, '-ManagedSettingsPath', $noManaged)
    $r = Invoke-Helper @('install', '-Harness', 'codex', '-Binary', $bin2, '-SettingsPath', $hooksFile, '-ManagedSettingsPath', $noManaged)
    $ours = Get-Ours $hooksFile
    Check ($r.Code -eq 0 -and $ours.Count -eq 1 -and $ours[0][1].command -ceq "& '$bin2' hook codex") 'binary change replaces the command' $r.Out
    Check ($r.Out -match 're-approv' -and $r.Out -match '/hooks') 'binary change says re-approval is needed' $r.Out

    # 4. trust and enablement state is reported, never written
    $dir = Case 'state'; $hooksFile = Join-Path $dir 'hooks.json'; $configFile = Join-Path $dir 'config.toml'
    $r = Invoke-Helper @('install', '-Harness', 'codex', '-Binary', $bin, '-SettingsPath', $hooksFile, '-ManagedSettingsPath', $noManaged)
    $key = $hooksFile + ':pre_tool_use:0:0'
    Write-Text $configFile ($config + "`n[hooks.state.'$key']`ntrusted_hash = `"sha256:abc`"`nenabled = false`n")
    $r = Invoke-Helper @('status', '-Harness', 'codex', '-SettingsPath', $hooksFile, '-ManagedSettingsPath', $noManaged)
    Check ($r.Out -match 'disabled in Codex') 'status reports a hook disabled in /hooks' $r.Out
    Write-Text $configFile ($config + "`n[hooks.state.'$key']`ntrusted_hash = `"sha256:abc`"`nenabled = true`n")
    $r = Invoke-Helper @('status', '-Harness', 'codex', '-SettingsPath', $hooksFile, '-ManagedSettingsPath', $noManaged)
    Check ($r.Out -match 'trust recorded') 'status reports recorded trust' $r.Out

    # 5. policy: feature flag in config.toml and managed requirements.toml
    $policies = @(
        @{ Name = 'quoted feature table'; Config = '["features"]' + "`nhooks = false"; Req = $null; Expect = 'hooks = false' },
        @{ Name = 'quoted feature key'; Config = '[features]' + "`n" + '"hooks" = false'; Req = $null; Expect = 'hooks = false' },
        @{ Name = 'quoted dotted key'; Config = 'features."hooks" = false'; Req = $null; Expect = 'hooks = false' },
        @{ Name = 'quoted inline feature'; Config = '"features" = {"hooks" = false}'; Req = $null; Expect = 'hooks = false' },
        @{ Name = 'escaped inline feature'; Config = 'features = {"h\u006foks" = false}'; Req = $null; Expect = 'hooks = false' },
        @{ Name = 'quoted managed hooks'; Config = $config; Req = '"hooks" = false'; Expect = 'requirements' },
        @{ Name = 'quoted managed-only'; Config = $config; Req = '"allow_managed_hooks_only" = true'; Expect = 'allow_managed_hooks_only' },
        @{ Name = 'config [features] hooks = false'; Config = "[features]`nhooks = false # off`n"; Req = $null; Expect = 'hooks = false' },
        @{ Name = 'config features.hooks = false'; Config = "features.hooks = false`n"; Req = $null; Expect = 'hooks = false' },
        @{ Name = 'requirements [features] hooks = false'; Config = $config; Req = "[features]`nhooks = false`n"; Expect = 'requirements' },
        @{ Name = 'requirements allow_managed_hooks_only'; Config = $config; Req = "allow_managed_hooks_only = true`n"; Expect = 'allow_managed_hooks_only' }
    )
    $i = 0
    foreach ($p in $policies) {
        $i++; $dir = Case "policy-$i"; $hooksFile = Join-Path $dir 'hooks.json'
        Write-Text (Join-Path $dir 'config.toml') $p.Config
        $managed = $noManaged
        if ($p.Req) { $managed = Join-Path $dir 'managed/requirements.toml'; Write-Text $managed $p.Req }
        $r = Invoke-Helper @('install', '-Harness', 'codex', '-Binary', $bin, '-SettingsPath', $hooksFile, '-ManagedSettingsPath', $managed)
        Check ($r.Code -ne 0 -and -not (Test-Path -LiteralPath $hooksFile) -and $r.Out.Contains($p.Expect)) "refused under $($p.Name)" $r.Out
        Write-Text $hooksFile $neighbour
        $prior = Get-Bytes $hooksFile
        $r = Invoke-Helper @('install', '-Harness', 'codex', '-Binary', $bin, '-SettingsPath', $hooksFile, '-ManagedSettingsPath', $managed)
        Check ($r.Code -ne 0 -and (Get-Bytes $hooksFile) -eq $prior) "policy preserves existing bytes: $($p.Name)" $r.Out
    }
    $dir = Case 'policy-allowed'; $hooksFile = Join-Path $dir 'hooks.json'
    $managed = Join-Path $dir 'managed/requirements.toml'; Write-Text $managed "allow_managed_hooks_only = false`n[features]`nhooks = true`n"
    $r = Invoke-Helper @('install', '-Harness', 'codex', '-Binary', $bin, '-SettingsPath', $hooksFile, '-ManagedSettingsPath', $managed)
    Check ($r.Code -eq 0) 'permissive requirements.toml allows install' $r.Out

    # 6. inline [hooks.<Event>] tables: Codex warns about two representations, so say so
    $dir = Case 'inline'; $hooksFile = Join-Path $dir 'hooks.json'
    Write-Text (Join-Path $dir 'config.toml') ($config + "`n[[hooks.Stop]]`n[[hooks.Stop.hooks]]`ntype = `"command`"`ncommand = `"echo`"`n")
    $r = Invoke-Helper @('install', '-Harness', 'codex', '-Binary', $bin, '-SettingsPath', $hooksFile, '-ManagedSettingsPath', $noManaged)
    Check ($r.Code -eq 0 -and $r.Out -match 'both') 'inline hook tables produce a coexistence note' $r.Out

    # 7. binary paths that would break the single-quoted command are refused
    foreach ($bad in @("C:\it's\mmhmem.exe", ("C:\a" + [char]0x2019 + "b\mmhmem.exe"), 'mmhmem.exe')) {
        $dir = Case ('bad-' + [Guid]::NewGuid().ToString('N').Substring(0, 6)); $hooksFile = Join-Path $dir 'hooks.json'
        $r = Invoke-Helper @('install', '-Harness', 'codex', '-Binary', $bad, '-SettingsPath', $hooksFile, '-ManagedSettingsPath', $noManaged)
        Check ($r.Code -ne 0 -and -not (Test-Path -LiteralPath $hooksFile)) "refuses binary path $bad" $r.Out
    }

    # 8. user scope only: a project's .codex directory is never a target
    $project = Case 'project'
    $r = Invoke-Helper @('install', '-Harness', 'codex', '-Binary', $bin, '-SettingsPath', (Join-Path $project '.codex/hooks.json'), '-ManagedSettingsPath', $noManaged) -CodexHome (Join-Path $scratch 'elsewhere')
    Check ($r.Code -ne 0 -and -not (Test-Path -LiteralPath (Join-Path $project '.codex'))) 'project .codex/hooks.json refused' $r.Out
    $r = Invoke-Helper @('install', '-Harness', 'codex', '-Binary', $bin, '-SettingsPath', (Join-Path $project 'config.toml'), '-ManagedSettingsPath', $noManaged)
    Check ($r.Code -ne 0 -and -not (Test-Path -LiteralPath (Join-Path $project 'config.toml'))) 'config.toml is never a target' $r.Out

    # 9. show and dry run
    $dir = Case 'show'; $hooksFile = Join-Path $dir 'hooks.json'
    $r = Invoke-Helper @('show', '-Harness', 'codex', '-Binary', $bin, '-SettingsPath', $hooksFile, '-ManagedSettingsPath', $noManaged)
    Check ($r.Code -eq 0 -and $r.Out.Contains($hooksFile) -and $r.Out.Contains('hook codex') -and $r.Out -match 'Remove it with' -and $r.Out -match 'shell command') 'show prints file, entry, behaviour and removal' $r.Out
    $r = Invoke-Helper @('install', '-Harness', 'codex', '-Binary', $bin, '-SettingsPath', $hooksFile, '-ManagedSettingsPath', $noManaged, '-DryRun')
    Check ($r.Code -eq 0 -and -not (Test-Path -LiteralPath $hooksFile) -and $r.Out -match 'dry-run') 'dry run writes nothing' $r.Out

    # 10. the Claude Code row is unaffected by a Codex entry in the same file shape
    $dir = Case 'cross'; $hooksFile = Join-Path $dir 'hooks.json'
    $r = Invoke-Helper @('install', '-Harness', 'codex', '-Binary', $bin, '-SettingsPath', $hooksFile, '-ManagedSettingsPath', $noManaged)
    $r = Invoke-Helper @('remove', '-Harness', 'claude', '-SettingsPath', $hooksFile)
    Check ((Get-Ours $hooksFile).Count -eq 1) 'claude removal ignores the Codex entry' $r.Out
} finally {
    Remove-Item -LiteralPath $scratch -Recurse -Force -ErrorAction SilentlyContinue
}
if ($failures.Count) { throw "manage_hooks codex tests failed ($($failures.Count)): $($failures -join '; ')" }
Write-Host "PASS: manage_hooks Codex entry ($($PSVersionTable.PSEdition) $($PSVersionTable.PSVersion))"
