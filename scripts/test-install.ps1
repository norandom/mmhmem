# Offline regression checks: bootstrap downloads are replaced with local fixtures.
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$scratch = Join-Path $repo 'tmp/install-tests'
New-Item -ItemType Directory -Force -Path $scratch | Out-Null
$originalTemp = $env:TEMP
$originalTmp = $env:TMP
$originalLocal = $env:LOCALAPPDATA
$originalPath = $env:PATH
[IO.File]::WriteAllText((Join-Path $scratch 'codex.cmd'), '@exit /b 0')
$env:PATH = $scratch + ';' + $env:PATH
$env:TEMP = $scratch
$env:TMP = $scratch
$env:LOCALAPPDATA = Join-Path $scratch 'profile'
$script:downloadBinary = $false
function Invoke-WebRequest {
    param([switch]$UseBasicParsing, [Parameter(Position=0)][string]$Uri, [string]$OutFile)
    if ($Uri -like '*/releases/latest/download/mmhmem.exe') {
        $script:downloadBinary = $true
        throw 'Dry run attempted to download a binary'
    }
    $relative = ($Uri -split '/HEAD/')[1]
    if (-not $relative) { throw "Unexpected URL: $Uri" }
    Copy-Item -LiteralPath (Join-Path $repo $relative) -Destination $OutFile
}
try {
    $output = & (Join-Path $repo 'install.ps1') -DryRun -Agent codex 6>&1 | Out-String
    if ($output -notmatch 'copy SKILL.md' -or $output -notmatch 'mmhmem-recap' -or $output -notmatch 'config.toml') {
        throw "Dry run missed registration, skill or recap: $output"
    }
    if ($script:downloadBinary -or (Test-Path $env:LOCALAPPDATA)) { throw 'Dry run wrote installation state' }
    $output = & (Join-Path $repo 'install.ps1') -Remove -DryRun -Agent codex 6>&1 | Out-String
    if ($output -notmatch 'local mode removed' -or $output -notmatch 'unified-memory') { throw "Removal failed: $output" }
    if (@(Get-ChildItem $scratch -Directory -Filter 'mmhmem-install-*').Count -ne 0) { throw 'Bootstrap leaked staging files' }
    $tokens = $null; $errors = $null
    foreach ($file in @('install.ps1','deploy/install_local.ps1','lib.ps1','build-marketplace.ps1')) {
        [Management.Automation.Language.Parser]::ParseFile((Join-Path $repo $file), [ref]$tokens, [ref]$errors) | Out-Null
        if ($errors.Count) { throw "$file parse errors: $errors" }
    }
    Write-Host 'PASS: PowerShell syntax, checkout-free dry run, Codex skill/recap, removal without companions, staging cleanup'
} finally {
    $env:TEMP = $originalTemp
    $env:TMP = $originalTmp
    $env:LOCALAPPDATA = $originalLocal
    $env:PATH = $originalPath
}
