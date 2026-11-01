#Requires -Version 5.1
<#
.SYNOPSIS
    Runs a command under a hard timeout and kills the whole process tree on
    expiry, so a stuck test/build can never block the session again.

.DESCRIPTION
    Usage:
        powershell -NoProfile -File scripts\with-timeout.ps1 -Seconds 900 `
            -Command cargo -ArgString "nextest run"

    Exit codes: the command's own exit code, or 124 on timeout (after
    taskkill /T /F kills the tree).
#>
param(
    [Parameter(Mandatory = $true)][int]$Seconds,
    [Parameter(Mandatory = $true)][string]$Command,
    [string]$ArgString = ''
)
$ErrorActionPreference = 'Continue'
$proc = Start-Process -FilePath $Command -ArgumentList $ArgString -NoNewWindow -PassThru
$timer = [System.Diagnostics.Stopwatch]::StartNew()
while (-not $proc.HasExited) {
    if ($timer.Elapsed.TotalSeconds -ge $Seconds) {
        taskkill /PID $proc.Id /T /F 2>$null | Out-Null
        Write-Host "TIMEOUT after ${Seconds}s: $Command $ArgString" -ForegroundColor Red
        exit 124
    }
    Start-Sleep -Milliseconds 250
}
Write-Host "OK in $([math]::Round($timer.Elapsed.TotalSeconds, 1))s: $Command $ArgString"
exit $proc.ExitCode
