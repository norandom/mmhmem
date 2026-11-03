$ErrorActionPreference='Stop'
$root=Split-Path -Parent $PSScriptRoot
$engine=[IO.File]::ReadAllText((Join-Path $root 'deploy/install_local.ps1'))
$a=$engine.IndexOf('$CodexBegin ='); $b=$engine.IndexOf('if (-not $Agent -and $Interactive)', $a)
. ([scriptblock]::Create($engine.Substring($a,$b-$a)))
function Info($message) {}
$DryRun=$false
$CodexCfgDir=Join-Path $root ('tmp/codex-registration-'+[guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force $CodexCfgDir|Out-Null
$cfg=Join-Path $CodexCfgDir 'config.toml'
$binary='C:\Program Files\mmhmem\mmhmem.exe'
$prefix="model = `"preserveä日`"`n"
$tail="[mcp_servers.other]`ncommand = `"other`"`n"
$block=Codex-Block $binary
foreach($text in @('', $prefix, $block, ($prefix+$block), ($block+"`n"+$tail), ($prefix+$block+"`n"+$tail), ($prefix+$block+"`n"+$block), ($block.Replace('install_local.ps1','install_local.sh')))) {
  [IO.File]::WriteAllText($cfg,$text)
  Set-CodexRegistration $binary
  $first=[IO.File]::ReadAllText($cfg)
  Set-CodexRegistration $binary
  $second=[IO.File]::ReadAllText($cfg)
  if($first -cne $second -or ([regex]::Matches($second,'(?m)^\[mcp_servers\.mmhmem\]$')).Count -ne 1){throw 'Repeated install changed config or duplicated registration'}
  if($text.Contains('preserve') -and -not $second.Contains($prefix.Trim())){throw 'Lost unrelated root settings'}
  if($text.Contains('mcp_servers.other') -and -not $second.Contains($tail.Trim())){throw 'Lost unrelated server'}
  Remove-CodexRegistration
  if([IO.File]::ReadAllText($cfg).Contains('[mcp_servers.mmhmem]')){throw 'Removal retained managed block'}
}
foreach($text in @('[mcp_servers.mmhmem]', '[mcp_servers."mmhmem"]', "[mcp_servers.'mmhmem'.env]", ($CodexBegin+"`n[mcp_servers.mmhmem]"),$CodexEnd,($block.Replace('args = ["stdio"]','custom = true')), 'mcp_servers = { mmhmem = {} }', "[mcp_servers]`nmmhmem.command = `"existing`"")) {
  [IO.File]::WriteAllText($cfg,$text)
  $failed=$false
  try {Set-CodexRegistration $binary}catch{$failed=$true}
  if(-not $failed -or [IO.File]::ReadAllText($cfg) -cne $text){throw "Ambiguous config was changed: $text"}
}
Write-Host 'PASS: Codex repeated installs, edge blocks, cross-shell markers, unrelated settings, backups, removal, fail-closed ambiguity'
