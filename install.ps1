#requires -Version 5.1
# Latest release binary + repository installer, without a checkout.
[CmdletBinding()]
param(
    [string]$Agent,
    [string]$WorkerModel,
    [string]$WorkerUrl,
    [string]$WorkerApiKey,
    [switch]$Remove,
    [switch]$DryRun
)
$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') { throw 'Use install.sh on Linux or macOS.' }
$arch = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
if ($arch -ne 'AMD64') { throw "No Windows $arch release asset; Windows x64 is required." }
$raw = 'https://raw.githubusercontent.com/norandom/mmhmem/HEAD'
$release = 'https://github.com/norandom/mmhmem/releases/latest/download/mmhmem.exe'
$binary = Join-Path $env:LOCALAPPDATA 'Programs\mmhmem\mmhmem.exe'
$tempBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\', '/')
$stage = Join-Path $tempBase ('mmhmem-install-' + [Guid]::NewGuid().ToString('N'))
try {
    $files = @('install_local.ps1')
    if (-not $Remove) {
        $files += 'skills/unified-memory/SKILL.md', 'commands/claude/mmhmem-recap.md',
            'commands/opencode/mmhmem-recap.md', 'commands/codex/mmhmem-recap.md'
    }
    foreach ($file in $files) {
        $destination = Join-Path $stage "deploy/$file"
        New-Item -ItemType Directory -Force -Path (Split-Path $destination) | Out-Null
        Invoke-WebRequest -UseBasicParsing "$raw/deploy/$file" -OutFile $destination
    }
    if (-not $Remove) {
        if ($DryRun) { Write-Host "[dry-run] download $release -> $binary" }
        else {
            $download = Join-Path $stage 'mmhmem.exe'
            Invoke-WebRequest -UseBasicParsing $release -OutFile $download
            if ((Get-Item -LiteralPath $download).Length -eq 0) { throw 'Empty binary download.' }
            New-Item -ItemType Directory -Force -Path (Split-Path $binary) | Out-Null
            try { Move-Item -LiteralPath $download -Destination $binary -Force }
            catch { throw "Cannot replace $binary. Close mmhmem harnesses and re-run. $_" }
        }
    }
    $engineArgs = @('-NoProfile', '-NonInteractive', '-File', (Join-Path $stage 'deploy/install_local.ps1'),
        '-Global', '-NonInteractive', '-Binary', $binary)
    foreach ($key in @('Agent', 'WorkerModel', 'WorkerUrl', 'WorkerApiKey')) {
        if ($PSBoundParameters.ContainsKey($key)) { $engineArgs += "-$key", $PSBoundParameters[$key] }
    }
    if ($Remove) { $engineArgs += '-Remove' }
    if ($DryRun) { $engineArgs += '-DryRun' }
    # The engine calls exit: isolate it from an irm | iex caller's console.
    $shell = if ($PSVersionTable.PSEdition -eq 'Core') { 'pwsh.exe' } else { 'powershell.exe' }
    & (Join-Path $PSHOME $shell) @engineArgs
    if ($LASTEXITCODE -ne 0) { throw "Installer failed (exit $LASTEXITCODE)." }
} finally {
    if ((Split-Path -Parent ([IO.Path]::GetFullPath($stage))).TrimEnd('\', '/') -ne $tempBase) {
        throw 'Unsafe installer cleanup path.'
    }
    if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
}
