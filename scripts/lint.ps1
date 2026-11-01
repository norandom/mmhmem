#Requires -Version 5.1
<#
.SYNOPSIS
    Local blocking lint/metrics gate for mmhmem (task 1.2; Req 13.1, 13.2, 13.5, 13.6).

.DESCRIPTION
    Runs the blocking checks locally, mirroring the CI gate (the CI workflow file
    itself is task 14.1's deliverable and is intentionally not created here):

      1. cargo fmt --check
      2. cargo clippy --all-targets -- -D warnings
         clippy.toml at the repo root caps cyclomatic complexity at 10 and
         cognitive complexity at 15 per function (Req 13.1 / 13.2).

    Plus an informational tokei per-file LOC report when tokei is installed
    (Req 13.5: dev tool only, never a runtime dependency; skipped otherwise).

    Exits 0 when all blocking checks pass, 1 otherwise.
#>
# NOTE: $ErrorActionPreference stays 'Continue' on purpose: on Windows
# PowerShell 5.1 a native command writing to stderr (cargo progress output)
# surfaces as an ErrorRecord that would abort the script under 'Stop'.
# Gate decisions below use $LASTEXITCODE explicitly instead.
$ErrorActionPreference = 'Continue'
Set-Location (Join-Path $PSScriptRoot '..')

$failed = $false

Write-Host '==> [1/2] cargo fmt --check'
cargo fmt --check
if ($LASTEXITCODE -ne 0) {
    $failed = $true
    Write-Host '    FAIL: cargo fmt --check (run ''cargo fmt'')' -ForegroundColor Red
} else {
    Write-Host '    PASS: cargo fmt --check' -ForegroundColor Green
}

Write-Host '==> [2/3] cargo clippy --all-targets -- -D warnings (clippy.toml caps cognitive complexity <= 15)'
cargo clippy --all-targets -- -D warnings
if ($LASTEXITCODE -ne 0) {
    $failed = $true
    Write-Host '    FAIL: cargo clippy --all-targets -- -D warnings' -ForegroundColor Red
} else {
    Write-Host '    PASS: cargo clippy --all-targets -- -D warnings' -ForegroundColor Green
}

Write-Host '==> [3/3] tests via cargo nextest run (falls back to cargo test)'
# A stale daemon process from an interrupted run holds target\debug\mmhmem.exe
# and blocks the build — reap any stale ones first.
Get-Process mmhmem -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
$nextest = Get-Command cargo-nextest -ErrorAction SilentlyContinue
if ($null -ne $nextest) {
    cargo nextest run
} else {
    Write-Host '    cargo-nextest not installed; falling back to cargo test'
    cargo test
}
if ($LASTEXITCODE -ne 0) {
    $failed = $true
    Write-Host '    FAIL: tests' -ForegroundColor Red
} else {
    Write-Host '    PASS: tests' -ForegroundColor Green
}

# Informational size report (Req 13.5): tokei is a dev tool only and never a
# runtime dependency; when it is not installed the report is simply skipped.
$tokei = Get-Command tokei -ErrorAction SilentlyContinue
if ($null -ne $tokei) {
    Write-Host '==> tokei (informational per-file LOC report, non-blocking)'
    tokei
} else {
    Write-Host '==> tokei not installed; informational LOC report skipped (optional dev tool, Req 13.5)'
}

if ($failed) {
    Write-Host 'LINT GATE: FAILED' -ForegroundColor Red
    exit 1
}
Write-Host 'LINT GATE: PASSED' -ForegroundColor Green
exit 0
