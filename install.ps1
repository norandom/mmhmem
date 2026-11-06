# mmhmem: this installer has moved.
#
# This file only prints the new install command. It downloads nothing, changes
# nothing and ignores every option (old ones such as -Agent or -Hooks are now
# chosen in mmhmem setup). The real installer is a per-release asset:
#   irm https://github.com/norandom/mmhmem/releases/latest/download/install.ps1 | iex
#
# Exit code 2 means "nothing was installed, use the new command". Run as a file
# (.\install.ps1 or powershell -File install.ps1) it exits with 2. Piped into
# iex it never calls exit, because that would close your terminal; it sets
# $LASTEXITCODE to 2 instead.
function Show-MmhmemMoved {
    Write-Host 'The mmhmem installer has moved. This script did not download or change anything.'
    Write-Host ''
    Write-Host 'To install mmhmem, run this in PowerShell:'
    Write-Host ''
    Write-Host '  irm https://github.com/norandom/mmhmem/releases/latest/download/install.ps1 | iex'
    Write-Host ''
    Write-Host 'It installs the latest signed release and opens mmhmem setup, where you choose'
    Write-Host 'your tools and features. Options such as -Agent or -Hooks are no longer used.'
    Write-Host ''
    Write-Host 'Updating? With mmhmem v0.5.1 or later run: mmhmem update'
    Write-Host 'With v0.5.0 run: mmhmem setup. Older versions: use the command above.'
}
Show-MmhmemMoved
# A function defined by a script file knows that file; one defined through iex does not.
if (${function:Show-MmhmemMoved}.File) {
    Remove-Item -LiteralPath function:Show-MmhmemMoved
    exit 2
}
Remove-Item -LiteralPath function:Show-MmhmemMoved
$global:LASTEXITCODE = 2
