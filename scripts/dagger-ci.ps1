#Requires -Version 5.1
<#
.SYNOPSIS
    Run the canonical container CI pipeline from the native Windows host.
.EXAMPLE
    powershell -NoProfile -File scripts/dagger-ci.ps1 -Call static-release -ExportPath tmp/verified-linux
.DESCRIPTION
    Capture provenance before the sanctioned source sync. Linux checks execute
    only in Dagger containers; WSL transports the module and exports results to
    repository scratch. TLS configuration and TLS-specific checks are excluded.
#>
param([string]$Call = 'all', [string[]]$ExtraArgs = @(), [string]$ExportPath = '')

$script:CiRepository = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path

function Get-CiExportPath([string]$RequestedPath) {
    if (-not $RequestedPath) { return '' }
    $full = [System.IO.Path]::GetFullPath($(if ([System.IO.Path]::IsPathRooted($RequestedPath)) { $RequestedPath } else { Join-Path $script:CiRepository $RequestedPath }))
    $scratch = [System.IO.Path]::GetFullPath((Join-Path $script:CiRepository 'tmp'))
    if (-not $full.StartsWith($scratch + [System.IO.Path]::DirectorySeparatorChar, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw 'Exports must be a new directory beneath repository tmp/'
    }
    $cursor = $full
    while ($cursor.Length -gt $script:CiRepository.Length) {
        $item = Get-Item -LiteralPath $cursor -Force -ErrorAction SilentlyContinue
        if ($item -and ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint)) { throw 'Export paths must not traverse a reparse point' }
        $cursor = Split-Path -Parent $cursor
    }
    if (Test-Path -LiteralPath $full) { throw 'Export destination already exists; choose a fresh directory' }
    return $full
}

function Invoke-MmhmemCi {
    param([string]$Call = 'all', [string[]]$ExtraArgs = @(), [string]$ExportPath = '')
    $ErrorActionPreference = 'Stop'
    if ($Call -notmatch '^[a-z][a-z0-9-]*$') { throw 'Invalid Dagger function name' }
    foreach ($argument in $ExtraArgs) {
        if ($argument -notmatch '^--[a-z][a-z0-9-]*(=[A-Za-z0-9_./:\\-]+)?$' -or
            $argument -match '^--(revision|validation-id|source-clean|src|output|module|help)(=|$)') {
            throw 'ExtraArgs accepts safe function options only; source, provenance and export are wrapper-owned'
        }
    }
    if ($ExportPath -and $Call -notin @('all', 'static-release')) { throw 'ExportPath requires all or static-release' }
    $destination = Get-CiExportPath $ExportPath
    $provenanceCalls = @('all', 'release', 'deploy-artifacts', 'static-gate', 'static-release', 'static-installer')
    $freshCalls = @('runtime-setup', 'static-build', 'static-inspection', 'bundle-check', 'local-probe', 'server-probe', 'static-gate-tests') + $provenanceCalls
    $parameters = @($ExtraArgs)
    if ($Call -in $provenanceCalls) {
        $revision = ((& git -C $script:CiRepository rev-parse --verify HEAD) -join '').Trim()
        if ($LASTEXITCODE -ne 0 -or $revision -notmatch '^[0-9a-f]{40}$') { throw 'Cannot capture full repository revision' }
        $state = & git -C $script:CiRepository status --porcelain --untracked-files=normal
        if ($LASTEXITCODE -ne 0) { throw 'Cannot capture source cleanliness' }
        $clean = -not (($state -join "`n").Trim())
        $parameters += @(("--revision=$revision"), ('--source-clean=' + $clean.ToString().ToLowerInvariant()))
    }
    if ($Call -in $freshCalls) {
        $validationId = $Call + '-' + [Guid]::NewGuid().ToString('N')
        $parameters += '--validation-id=' + $validationId
    }
    if ($Call -in $provenanceCalls) { Write-Host "CI provenance: revision=$revision source_clean=$clean validation_id=$validationId" }
    $linuxSource = ((& wsl -d Debian -e wslpath -a -u $script:CiRepository) -join '').Trim()
    if ($LASTEXITCODE -ne 0 -or -not $linuxSource.StartsWith('/')) { throw 'Cannot resolve source path for CI transport' }
    $linuxDestination = '-'
    if ($destination) {
        $linuxDestination = ((& wsl -d Debian -e wslpath -a -u $destination) -join '').Trim()
        if ($LASTEXITCODE -ne 0 -or -not $linuxDestination.StartsWith('/')) { throw 'Cannot resolve export path for CI transport' }
    }
    # Each value remains a positional argument to the checked-in CI transport.
    # No source path, extra option or destination is interpolated into bash -c.
    $arguments = @('-d', 'Debian', '-e', 'bash', '--', "$linuxSource/scripts/dagger-ci.sh", $linuxSource, $Call, $linuxDestination) + $parameters
    $previousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $global:LASTEXITCODE = 1
        & wsl @arguments | Out-Host
        $code = $LASTEXITCODE
    } finally { $ErrorActionPreference = $previousPreference }
    if ($code -eq 0 -and $destination) {
        $manifest = Get-Content -Raw -LiteralPath (Join-Path $destination 'verification.json') | ConvertFrom-Json
        $actual = (Get-FileHash -LiteralPath (Join-Path $destination 'mmhmem-linux-x86_64') -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($manifest.revision -ne $revision -or $manifest.validation_id -ne $validationId -or
            $manifest.source_clean -ne $clean -or $manifest.publishable -ne $clean -or $manifest.artifact.sha256 -ne $actual) {
            throw 'Exported bundle identity/checksum does not match native provenance'
        }
        Write-Host "Verified bundle exported: $destination (sha256=$actual, publishable=$($manifest.publishable))"
    }
    return [int]$code
}

if ($MyInvocation.InvocationName -ne '.') { exit (Invoke-MmhmemCi @PSBoundParameters) }
