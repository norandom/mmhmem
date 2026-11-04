# Opt-in read-routing hook for the installers (Windows). Only manage_hooks.ps1 edits
# harness settings; nothing here writes a harness trust or approval record, and the
# helper is never given -ManagedSettingsPath (that would skip the registry policy check).

function Get-HookHarnesses([string]$List) {
    $out = @()
    foreach ($name in @(($List -split ',') | ForEach-Object { $_.Trim().ToLower() } | Where-Object { $_ })) {
        switch ($name) {
            'claude' { $out += 'claude' }
            'codex' { $out += 'codex' }
            'all' { $out += @('claude', 'codex') }
            'opencode' { throw "-Hooks $name is not available yet; $name currently gets advisory guidance only." }
            default { throw "unknown hooks harness '$name' (valid: claude, codex, all)" }
        }
    }
    return @($out | Select-Object -Unique)
}

# Runs the helper in a child shell; returns the exit code and its output lines.
function Invoke-HookHelper([string]$Script, [string[]]$HelperArgs) {
    $shell = if ($PSVersionTable.PSEdition -eq 'Core') { 'pwsh.exe' } else { 'powershell.exe' }
    $old = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        # Windows PowerShell 5.1 wraps native stderr as error records: keep only the text.
        $lines = @(& (Join-Path $PSHOME $shell) -NoProfile -ExecutionPolicy Bypass -File $Script @HelperArgs 2>&1 | ForEach-Object { [string]$_ })
        $code = $LASTEXITCODE
    } finally { $ErrorActionPreference = $old }
    return [pscustomobject]@{ Code = $code; Lines = $lines }
}
# The helper names its own path in "Remove it with:"; when it runs from the staging copy that path is
# deleted after install, so show the removal that will exist (the persistent helper, or the installer).
function Write-HookLines($Result, [string]$Destination, [string]$Harness = 'claude') {
    foreach ($line in $Result.Lines) {
        if ($Destination -and $line -match '^(\[mmhmem\] )?Remove it with:') {
            $prefix = $Matches[1]
            Write-Host ($prefix + 'After install, remove it with: powershell -NoProfile -ExecutionPolicy Bypass -File "' + $Destination + '" remove -Harness ' + $Harness)
            Write-Host ($prefix + 'Or re-run the installer with -RemoveHooks (a full -Remove removes it too).')
        } else { Write-Host $line }
    }
}

# Prints the exact entry, the file, what the hook sees and decides, and the removal command.
function Show-ReadRoutingHook([string]$Source, [string]$Binary, [string]$Destination, [string]$Harness = 'claude') {
    $result = Invoke-HookHelper $Source @('show', '-Harness', $Harness, '-Binary', $Binary)
    Write-HookLines $result $Destination $Harness
    return $result.Code
}

# Refuses before any write when user or managed policy disables hooks (dry run, output kept on failure only).
function Test-ReadRoutingHookAllowed([string]$Source, [string]$Binary, [string]$Harness = 'claude') {
    $result = Invoke-HookHelper $Source @('install', '-Harness', $Harness, '-Binary', $Binary, '-DryRun')
    if ($result.Code -ne 0) { Write-HookLines $result; Write-HookRefused }
    return $result.Code -eq 0
}

function Add-ReadRoutingHook([string]$Source, [string]$Destination, [string]$Binary, [switch]$DryRun, [string]$Harness = 'claude') {
    $script = $Source
    $helperArgs = @('install', '-Harness', $Harness, '-Binary', $Binary)
    if ($DryRun) {
        Write-Host "[mmhmem] [dry-run] copy manage_hooks.ps1 -> $Destination"
        $helperArgs += '-DryRun'
    } else {
        # Persistent copy: the removal command the helper prints names this path.
        New-Item -ItemType Directory -Path (Split-Path -Parent $Destination) -Force | Out-Null
        Copy-Item -LiteralPath $Source -Destination $Destination -Force
        $script = $Destination
    }
    $result = Invoke-HookHelper $script $helperArgs
    Write-HookLines $result $(if ($DryRun) { $Destination }) $Harness
    if ($result.Code -ne 0) { Write-HookRefused }
    return $result.Code
}

function Remove-ReadRoutingHook([string]$Source, [string]$Destination, [switch]$DryRun, [string[]]$Harnesses = @('claude', 'codex')) {
    foreach ($harness in $Harnesses) {
        $helperArgs = @('remove', '-Harness', $harness)
        if ($DryRun) { $helperArgs += '-DryRun' }
        $result = Invoke-HookHelper $Source $helperArgs
        Write-HookLines $result
        if ($result.Code -ne 0) { return $result.Code }
    }
    if ($Harnesses.Count -gt 1 -and (Test-Path -LiteralPath $Destination)) {
        if ($DryRun) { Write-Host "[mmhmem] [dry-run] rm $Destination" }
        else { Remove-Item -LiteralPath $Destination -Force }
    }
    return 0
}

function Write-HookRefused {
    Write-Host '[mmhmem] This harness gets advisory guidance only (the mmhmem routing rule): its hooks are disabled or restricted by the policy named above.'
}
function Write-HookAdvisory([string]$Harness = 'claude') {
    if ($Harness -eq 'codex') { Write-Host '[mmhmem] Start Codex and approve the new hook in /hooks; until trusted it is skipped. Existing trust and enablement state was not changed.' }
    else { Write-Host '[mmhmem] Start a new Claude Code session: hooks are read at startup. Accept the workspace trust dialog if asked and review the hook in /hooks.' }
    Write-Host '[mmhmem] OpenCode currently gets advisory guidance only (its hook is not available yet).'
}
