#requires -Version 5.1
# Latest release binary + repository installer, without a checkout.
[CmdletBinding()]
param(
    [string]$Agent,
    [string]$WorkerModel,
    [string]$WorkerUrl,
    [string]$WorkerApiKey,
    [string]$Hooks,
    [switch]$RemoveHooks,
    [switch]$Remove,
    [switch]$DryRun
)
$ErrorActionPreference = 'Stop'
$interactive = -not [Console]::IsInputRedirected
if (-not $Agent -and -not $RemoveHooks) {
    if (-not $interactive) { throw 'Agent selection needs a terminal. Pass -Agent codex (or -Agent all) for unattended use.' }
    if ($Host.UI.SupportsVirtualTerminal -and -not $env:NO_COLOR -and $env:TERM -ne 'dumb') {
        try {
            $art = Invoke-WebRequest -UseBasicParsing 'https://raw.githubusercontent.com/norandom/mmhmem/HEAD/deploy/banner.ansi'
            Write-Host ([Text.Encoding]::UTF8.GetString($art.RawContentStream.ToArray()))
        } catch { } # Decoration must never prevent installation.
    }
    Write-Host "`n  mmhmem  /  shared memory for your agents" -ForegroundColor Cyan
    Write-Host '  ----------------------------------------'
    if (-not $Remove) {
        do {
            Write-Host "`n  [1] Install or update`n  [2] Remove integrations`n  [q] Cancel`n"
            $choice = Read-Host '  Choose an action'
            if ($choice -eq 'q') { return }
        } until ($choice -in @('1', '2'))
        if ($choice -eq '2') { $Remove = [switch]$true }
    }
}
if ($env:OS -ne 'Windows_NT') { throw 'Use install.sh on Linux or macOS.' }
$arch = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
if ($arch -ne 'AMD64') { throw "No Windows $arch release asset; Windows x64 is required." }
$raw = 'https://raw.githubusercontent.com/norandom/mmhmem/HEAD'
$release = 'https://github.com/norandom/mmhmem/releases/latest/download/mmhmem.exe'
$binary = Join-Path $env:LOCALAPPDATA 'Programs\mmhmem\mmhmem.exe'
$tempBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\', '/')
$stage = Join-Path $tempBase ('mmhmem-install-' + [Guid]::NewGuid().ToString('N'))
try {
    $files = @('install_local.ps1', 'manage_routing.ps1', 'routing_onboarding.ps1', 'hooks_onboarding.ps1', 'manage_hooks.ps1')
    $noBinary = $Remove -or $RemoveHooks
    if (-not $noBinary) {
        $files += 'opencode/mmhmem-read-routing.js', 'instructions/mmhmem-routing.md', 'install_binary.ps1', 'check_worker.ps1', 'skills/unified-memory/SKILL.md', 'codex/skills/mmhmem-recap/SKILL.md', 'commands/claude/mmhmem-recap.md',
            'commands/opencode/mmhmem-recap.md', 'commands/codex/mmhmem-recap.md'
    }
    foreach ($file in $files) {
        $destination = Join-Path $stage "deploy/$file"
        New-Item -ItemType Directory -Force -Path (Split-Path $destination) | Out-Null
        Invoke-WebRequest -UseBasicParsing "$raw/deploy/$file" -OutFile $destination
    }
    if (-not $noBinary) {
        if ($DryRun) { Write-Host "[dry-run] download $release -> $binary" }
        else {
            $download = Join-Path $stage 'mmhmem.exe'
            Invoke-WebRequest -UseBasicParsing $release -OutFile $download
            if ((Get-Item -LiteralPath $download).Length -eq 0) { throw 'Empty binary download.' }
        }
    }
    $engineArgs = @('-NoProfile', '-File', (Join-Path $stage 'deploy/install_local.ps1'),
        '-Global', '-Binary', $binary)
    if ($Agent -or $RemoveHooks) { $engineArgs += '-NonInteractive' }
    if (-not $noBinary -and -not $DryRun) { $engineArgs += '-InstallSource', $download }
    foreach ($key in @('Agent', 'WorkerModel', 'WorkerUrl', 'WorkerApiKey', 'Hooks')) {
        if ($PSBoundParameters.ContainsKey($key)) { $engineArgs += "-$key", $PSBoundParameters[$key] }
    }
    if ($Remove) { $engineArgs += '-Remove' }
    if ($RemoveHooks) { $engineArgs += '-RemoveHooks' }
    if ($DryRun) { $engineArgs += '-DryRun' }
    # The engine calls exit: isolate it from an irm | iex caller's console.
    $shell = if ($PSVersionTable.PSEdition -eq 'Core') { 'pwsh.exe' } else { 'powershell.exe' }
    & (Join-Path $PSHOME $shell) @engineArgs
    if ($LASTEXITCODE -ne 0) { throw "Installer failed (exit $LASTEXITCODE)." }
    if (-not $noBinary -and -not $DryRun -and (Test-Path -LiteralPath $binary)) {
        $binDirectory = Split-Path -Parent $binary
        if (($env:PATH -split ';') -notcontains $binDirectory) { $env:PATH += ';' + $binDirectory }
    }
} finally {
    if ((Split-Path -Parent ([IO.Path]::GetFullPath($stage))).TrimEnd('\', '/') -ne $tempBase) {
        throw 'Unsafe installer cleanup path.'
    }
    if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
}
