$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'runner.ps1')
$rejected = $false
try { Invoke-RoutingGoldenCases -Adapter { param($case,$source,$canonical) @{ Outcome='changed'; Bytes=[byte[]](1) } } }
catch { if ($_.Exception.Message -notmatch 'expected changed and exact bytes') { throw }; $rejected = $true }
if (-not $rejected) { throw 'Incorrect adapter accepted' }
Write-Host 'PASS: golden runner rejects incorrect bytes'
