# Shared by the checkout and checkout-free Windows installers.
function Install-MmhmemBinary {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$Source, [Parameter(Mandatory)][string]$Destination)
    $ErrorActionPreference = 'Stop'
    $directory = Split-Path -Parent $Destination
    New-Item -ItemType Directory -Force -Path $directory | Out-Null
    $id = [Guid]::NewGuid().ToString('N')
    $staged = "$Destination.$id.new"
    $backup = "$Destination.$id.old"
    $renamed = $false
    try {
        # Complete the copy on the destination volume before touching the old binary.
        Copy-Item -LiteralPath $Source -Destination $staged
        if ((Get-Item -LiteralPath $staged).Length -eq 0) { throw 'Empty binary download.' }
        if (Test-Path -LiteralPath $Destination) {
            # Windows permits renaming a running executable, but not overwriting it.
            [IO.File]::Move($Destination, $backup)
            $renamed = $true
        }
        try { [IO.File]::Move($staged, $Destination) }
        catch {
            if ($renamed) { [IO.File]::Move($backup, $Destination) }
            throw
        }
    } finally {
        if (Test-Path -LiteralPath $staged) { Remove-Item -LiteralPath $staged -Force }
    }
    # Keep the immediate previous version; retry older locked images next time.
    $oldVersions = @(Get-ChildItem -LiteralPath $directory -File | Where-Object {
        $_.Name -match ('^' + [regex]::Escape((Split-Path -Leaf $Destination)) + '\.[0-9a-f]{32}\.old$')
    } | Sort-Object LastWriteTimeUtc -Descending)
    $keep = if ($renamed) { $backup } elseif ($oldVersions.Count) { $oldVersions[0].FullName } else { '' }
    $oldVersions | Where-Object { $_.FullName -ne $keep } | ForEach-Object {
        try { Remove-Item -LiteralPath $_.FullName -Force }
        catch { Write-Host '[mmhmem] An older version is still running; cleanup will retry on the next update.' }
    }
    if ($renamed) { Write-Host '[mmhmem] Updated binary. Running agents continue using their current version until restarted.' }
}
