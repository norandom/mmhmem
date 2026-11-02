# Native Windows regression: update a running image without stopping its process.
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '../deploy/install_binary.ps1')
$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('mmhmem-update-test-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $testRoot | Out-Null
$running = $null
$locked = $null
try {
    $source = Join-Path $testRoot 'release.exe'
    $target = Join-Path $testRoot 'mmhmem.exe'
    Copy-Item -LiteralPath "$env:SystemRoot/System32/ping.exe" -Destination $source
    Install-MmhmemBinary $source $target
    $running = Start-Process -FilePath $target -ArgumentList @('-t', '127.0.0.1') -WindowStyle Hidden -PassThru
    Start-Sleep -Milliseconds 500
    if ($running.HasExited) { throw 'Running executable fixture exited early' }
    # Reproduce the reported error before applying the replacement strategy.
    $oldMove = Join-Path $testRoot 'old-move.exe'
    Copy-Item -LiteralPath $source -Destination $oldMove
    $failed = $false
    try { Move-Item -LiteralPath $oldMove -Destination $target -Force }
    catch { $failed = $true; Write-Host "Reproduced running-image overwrite failure: $_" }
    if (-not $failed) { throw 'Expected the old replacement method to fail against a running image' }
    [IO.File]::WriteAllText($source, 'new version')
    Install-MmhmemBinary $source $target
    if ([IO.File]::ReadAllText($target) -ne 'new version' -or $running.HasExited) {
        throw 'Update failed or terminated the running process'
    }
    # Consecutive updates must not collide with a retained running-image backup.
    [IO.File]::WriteAllText($source, 'next version')
    Install-MmhmemBinary $source $target
    if ([IO.File]::ReadAllText($target) -ne 'next version') { throw 'Repeat update failed' }
    Stop-Process -Id $running.Id -Force
    $running.WaitForExit()
    Install-MmhmemBinary $source $target
    if (@(Get-ChildItem $testRoot -Filter '*.old').Count) { throw 'Stopped image backup was not cleaned up' }
    # Invalid input and exclusive locks must preserve the installed bytes.
    [IO.File]::WriteAllText($source, '')
    $failed = $false
    try { Install-MmhmemBinary $source $target } catch { $failed = $true }
    if (-not $failed -or [IO.File]::ReadAllText($target) -ne 'next version') { throw 'Empty update damaged installation' }
    [IO.File]::WriteAllText($source, 'locked update')
    $locked = [IO.File]::Open($target, 'Open', 'Read', 'None')
    $failed = $false
    try { Install-MmhmemBinary $source $target } catch { $failed = $true }
    $locked.Dispose(); $locked = $null
    if (-not $failed -or [IO.File]::ReadAllText($target) -ne 'next version') { throw 'Locked update damaged installation' }
    if (@(Get-ChildItem $testRoot -Filter '*.new').Count) { throw 'Failed update leaked staging files' }
    Write-Host 'PASS: fresh install, running-image update, repeat update, backup cleanup, empty input, exclusive lock'
} finally {
    if ($locked) { $locked.Dispose() }
    if ($running -and -not $running.HasExited) { Stop-Process -Id $running.Id -Force; $running.WaitForExit() }
    $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\', '/')
    if ((Split-Path -Parent ([IO.Path]::GetFullPath($testRoot))).TrimEnd('\', '/') -ne $tempRoot) {
        throw 'Unsafe test cleanup path'
    }
    Remove-Item -LiteralPath $testRoot -Recurse -Force
}
