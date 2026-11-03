$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$RepoRoot = $repo
$engine = [IO.File]::ReadAllText((Join-Path $repo 'deploy/install_local.ps1'))
$start = $engine.IndexOf('function Install-Skill(')
$end = $engine.IndexOf('# Merges one JSON assignment', $start)
. ([scriptblock]::Create($engine.Substring($start, $end - $start)))
function Info($message) {}
function Want-Agent($agent) { return $agent -eq 'codex' }
function Set-CodexRegistration($binary) {}
function Remove-CodexRegistration {}
$RepoRoot = $repo
$SkillSrc = Join-Path $repo 'deploy/skills/unified-memory/SKILL.md'
$CodexCommandSrc = Join-Path $repo 'deploy/commands/codex/mmhmem-recap.md'
$testProfile = Join-Path $repo ('tmp/recap-skill-' + [guid]::NewGuid().ToString('N'))
$CodexCfgDir = Join-Path $testProfile '.codex'
$Registered = New-Object 'System.Collections.Generic.List[string]'
$HasCodex = $true; $Global = $true; $Remove = $false; $DryRun = $false
$start = $engine.IndexOf("if ((Want-Agent 'codex') -and `$HasCodex)")
$end = $engine.IndexOf('# --- copied binary', $start)
$block = [scriptblock]::Create($engine.Substring($start, $end - $start).Replace('$HOME', '$testProfile'))
$destination = Join-Path $testProfile '.agents/skills/mmhmem-recap/SKILL.md'
$source = Join-Path $repo 'deploy/codex/skills/mmhmem-recap/SKILL.md'
& $block
if ((Get-FileHash $destination).Hash -ne (Get-FileHash $source).Hash) { throw 'Recap skill not installed intact' }
[IO.File]::WriteAllText($destination, 'stale skill')
& $block
if ((Get-FileHash $destination).Hash -ne (Get-FileHash $source).Hash) { throw 'Recap skill not updated' }
$keep = Join-Path (Split-Path $destination) 'user.txt'
[IO.File]::WriteAllText($keep, 'preserve')
$Remove = $true
& $block
if (Test-Path $destination) { throw 'Recap skill not removed' }
if (-not (Test-Path $keep)) { throw 'Removal deleted unrelated skill files' }
Write-Host 'PASS: Codex recap skill installation, update, and selective removal'
