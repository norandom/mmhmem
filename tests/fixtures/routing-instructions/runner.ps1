# Dot-source this runner; Adapter receives case, source bytes (or null), canonical bytes.
# Return @{ Outcome = 'changed|unchanged|error|delete'; Bytes = byte[] or $null }.
function Invoke-RoutingGoldenCases {
    param([Parameter(Mandatory=$true)][scriptblock]$Adapter)
    $root = $PSScriptRoot
    $manifest = Get-Content -LiteralPath (Join-Path $root 'cases.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    $canonical = [IO.File]::ReadAllBytes((Join-Path $root $manifest.canonical))
    foreach ($case in $manifest.cases) {
        $source = $null
        $expected = $null
        if ($case.input) { $source = [IO.File]::ReadAllBytes((Join-Path $root $case.input)) }
        if ($case.expected) { $expected = [IO.File]::ReadAllBytes((Join-Path $root $case.expected)) }
        $result = & $Adapter $case $source $canonical
        $equal = ($null -eq $expected -and $null -eq $result.Bytes)
        if ($null -ne $expected -and $null -ne $result.Bytes) {
            $equal = [Convert]::ToBase64String($expected) -ceq [Convert]::ToBase64String([byte[]]$result.Bytes)
        }
        if ($result.Outcome -cne $case.outcome -or -not $equal) { throw "$($case.id): expected $($case.outcome) and exact bytes" }
    }
    return $manifest.cases.Count
}
