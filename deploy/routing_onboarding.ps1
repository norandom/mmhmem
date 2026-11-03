# Selected-harness routing preflight; no configuration writes occur here.
. (Join-Path $PSScriptRoot 'manage_routing.ps1')
function Assert-RoutingOverride {
 param([string]$Name,[string]$Expected)
 $value=[Environment]::GetEnvironmentVariable($Name)
 if (-not $value) { return }
 if ($Expected) {
  try { if ([IO.Path]::GetFullPath($value).TrimEnd('\','/') -ieq [IO.Path]::GetFullPath($Expected).TrimEnd('\','/')) { return } } catch {}
 }
 throw "$Name changes the effective harness configuration. Use the default configuration or configure mmhmem manually; no integrations were changed."
}
function New-HarnessRoutingPlan {
 param([string]$Harness,[string]$UserRoot,[string]$Project,[switch]$Global,[switch]$Remove)
 $base=if ($Global) { $UserRoot } else { $Project }
 if ($Harness -eq 'claude') {
  Assert-RoutingOverride 'CLAUDE_CONFIG_DIR' (Join-Path $UserRoot '.claude')
  $path=Join-Path $base '.claude/rules/mmhmem.md'
  if (-not $Remove) {
  foreach ($settings in @((Join-Path $UserRoot '.claude/settings.json'),(Join-Path $Project '.claude/settings.json'),(Join-Path $Project '.claude/settings.local.json'))) {
   if (Test-Path -LiteralPath $settings) {
    $json=[IO.File]::ReadAllText($settings) | ConvertFrom-Json
    if ($json.claudeMdExcludes) { throw 'claudeMdExcludes may exclude routing rules; review it before onboarding. No integrations were changed.' }
   }
  }
  foreach ($name in @('CLAUDE_CODE_SIMPLE','CLAUDE_CODE_DISABLE_PROJECT_INSTRUCTIONS')) { if ([Environment]::GetEnvironmentVariable($name)) { throw "$name disables or changes instruction loading; clear it before onboarding." } }
  }
 } else {
  Assert-RoutingOverride 'OPENCODE_CONFIG_DIR' (Join-Path $UserRoot '.config/opencode')
  Assert-RoutingOverride 'XDG_CONFIG_HOME' (Join-Path $UserRoot '.config')
  Assert-RoutingOverride 'OPENCODE_CONFIG' ''
  Assert-RoutingOverride 'OPENCODE_CONFIG_CONTENT' ''
  if (-not $Remove -and -not $Global -and $env:OPENCODE_DISABLE_PROJECT_CONFIG -match '^(1|true)$') { throw 'OPENCODE_DISABLE_PROJECT_CONFIG disables project instructions; clear it before onboarding.' }
  $path=if ($Global) { Join-Path $UserRoot '.config/opencode/AGENTS.md' } else { Join-Path $Project 'AGENTS.md' }
  if (-not $Remove -and -not (Test-Path -LiteralPath $path)) {
   $fallback=Join-Path (Split-Path -Parent $path) 'CLAUDE.md'
   if (Test-Path -LiteralPath $fallback) {
    $version=''; try { $version=(& opencode --version 2>$null | Out-String).Trim() } catch {}
    if ($version -notmatch '^v?([2-9]|[1-9][0-9]+)\.') { throw 'Creating AGENTS.md could suppress existing CLAUDE.md instructions in this OpenCode version; review the fallback manually.' }
   }
  }
 }
 $canonical=if ($Remove) { [byte[]]@() } else { [IO.File]::ReadAllBytes((Join-Path $PSScriptRoot 'instructions/mmhmem-routing.md')) }
 $operation=if ($Remove) { 'remove' } else { 'install' }
 return New-RoutingPlan -Path $path -Operation $operation -Canonical $canonical -Dedicated:($Harness -eq 'claude')
}
function Complete-HarnessRouting {
 param($Plan,[switch]$DryRun)
 try { Invoke-RoutingCommit -Plan $Plan -DryRun:$DryRun }
 catch { throw "Routing update failed at $($Plan.Path). Earlier integration operations may have completed; rerun after correcting this error. $($_.Exception.Message)" }
 $prefix=if ($DryRun) { '[dry-run] ' } else { '' }
 Write-Host "[mmhmem] $($prefix)routing instructions: $($Plan.Path) ($($Plan.Outcome))"
 Write-Host '[mmhmem] Start a fresh harness session to load updated instructions and MCP metadata.'
}
