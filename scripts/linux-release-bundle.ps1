#Requires -Version 5.1
# Native counterpart of ci/scripts/check_bundle.py for the draft publisher.
$script:LinuxAssetName='mmhmem-linux-x86_64'
$script:LinuxTarget='x86_64-unknown-linux-musl'
$script:LinuxRuntimeCases=@('pristine_startup','version','mcp_initialize','tools_list','persistence','stats_text','stats_json','server_health','server_auth')
function Get-ReleaseField($Record,[string]$Name,[string]$Kind='string') {
    $property=@($Record.PSObject.Properties | Where-Object {$_.Name -ceq $Name})
    if ($Record -isnot [pscustomobject] -or $property.Count -ne 1) {throw "Missing field: $Name"}
    $value=$property[0].Value
    $valid=switch ($Kind) {
        'string' {$value -is [string] -and -not [string]::IsNullOrWhiteSpace($value)}
        'boolean' {$value -is [bool]}
        'integer' {$value -is [int] -or $value -is [long]}
        'object' {$value -is [pscustomobject]}
        'array' {$value -is [array]}
    }
    if (-not $valid) {throw "Invalid $Name ($Kind required)"}
    return ,$value
}
function Read-ReleaseJson([string]$Path) {
    $raw=[IO.File]::ReadAllText($Path)
    if ($raw.Length -gt 8388608) {throw 'JSON evidence exceeds size limit'}
    $result=$raw | ConvertFrom-Json
    # ConvertFrom-Json on PS5.1 can overwrite duplicate keys. Inspect decoded
    # property names separately; JSON grammar itself is validated above.
    $stack=New-Object 'System.Collections.Generic.Stack[object]'
    foreach ($token in [regex]::Matches($raw,'"(?:\\.|[^"\\])*"|[{}\[\],:]')) {
        $value=$token.Value
        if ($value -eq '{') {$stack.Push(@{Object=$true;WantKey=$true;Keys=(New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::Ordinal))})}
        elseif ($value -eq '[') {$stack.Push(@{Object=$false})}
        elseif ($value -in @('}',']')) {$null=$stack.Pop()}
        elseif ($value -eq ',' -and $stack.Count -and $stack.Peek().Object) {$stack.Peek().WantKey=$true}
        elseif ($value.StartsWith('"') -and $stack.Count -and $stack.Peek().Object -and $stack.Peek().WantKey) {
            $key=$value | ConvertFrom-Json
            if (-not $stack.Peek().Keys.Add($key)) {throw "Duplicate JSON field: $key"}
            $stack.Peek().WantKey=$false
        }
    }
    if ($result -isnot [pscustomobject]) {throw 'JSON evidence must be an object'}
    return $result
}
function Test-ReleaseEqual($Left,$Right) {
    if ($null -eq $Left -or $null -eq $Right) {return ($null -eq $Left -and $null -eq $Right)}
    if ($Left -is [pscustomobject] -and $Right -is [pscustomobject]) {
        $leftNames=@($Left.PSObject.Properties.Name | Sort-Object); $rightNames=@($Right.PSObject.Properties.Name | Sort-Object)
        if (($leftNames -join "`n") -cne ($rightNames -join "`n")) {return $false}
        foreach ($key in $leftNames) {if (-not (Test-ReleaseEqual $Left.$key $Right.$key)) {return $false}}
        return $true
    }
    if ($Left -is [array] -and $Right -is [array]) {
        if ($Left.Count -ne $Right.Count) {return $false}
        for ($index=0;$index -lt $Left.Count;$index++) {if (-not (Test-ReleaseEqual $Left[$index] $Right[$index])) {return $false}}
        return $true
    }
    if (($Left -is [bool]) -ne ($Right -is [bool]) -or ($Left -is [string]) -ne ($Right -is [string])) {return $false}
    return ($Left -ceq $Right)
}
function Assert-ReleaseState($Record,[string]$Label,[switch]$Required) {
    if ($Required -and (Get-ReleaseField $Record 'status') -cne 'passed') {throw "$Label did not pass"}
    foreach ($field in @('status','error','errors','failure','failures','passed','success')) {
        $property=$Record.PSObject.Properties[$field]
        if (-not $property) {continue}
        if (($field -eq 'status' -and $property.Value -cne 'passed') -or
            ($field -in @('error','errors','failure','failures') -and $property.Value) -or
            ($field -in @('passed','success') -and ($property.Value -isnot [bool] -or -not $property.Value))) {throw "$Label has contradictory or failed evidence"}
    }
}
function Assert-ReleaseNames($Record,[string[]]$Names,[string]$Label) {
    if ($Record -isnot [pscustomobject] -or (@($Record.PSObject.Properties.Name | Sort-Object) -join "`n") -cne (@($Names | Sort-Object) -join "`n")) {throw "$Label must have exactly the required records"}
}
function Assert-ReleaseImage([string]$Value,[string]$Expected) {
    if ($Value -cnotmatch ('^(docker\.io/library/)?'+[regex]::Escape($Expected)+'@sha256:[0-9a-f]{64}$')) {throw "Invalid image identity for $Expected"}
}
function Get-ReleaseBundleFile([string]$Root,[string]$Relative,[switch]$Report) {
    if ($Relative -cnotmatch '^[A-Za-z0-9_.-]+(/[A-Za-z0-9_.-]+)*$' -or @($Relative.Split('/') | Where-Object {$_ -in @('.','..')}).Count -or ($Report -and -not $Relative.StartsWith('reports/'))) {throw "Unsafe bundle path: $Relative"}
    $path=$Root
    foreach ($part in $Relative.Split('/')) {
        $path=Join-Path $path $part
        $item=Get-Item -LiteralPath $path -Force -ErrorAction Stop
        if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {throw "Reparse bundle path: $Relative"}
    }
    if ($item.PSIsContainer) {throw "Bundle file missing: $Relative"}
    return $path
}
function Assert-ReleaseReportIdentity($Report,$Manifest,[switch]$ValidationRequired) {
    Assert-ReleaseState $Report 'report' -Required
    foreach ($key in @('revision','target','validation_id')) {
        if ($Report.PSObject.Properties[$key] -and -not (Test-ReleaseEqual $Report.$key $Manifest.$key)) {throw "Report $key does not match bundle"}
    }
    if ($ValidationRequired -and (Get-ReleaseField $Report 'validation_id') -cne $Manifest.validation_id) {throw 'Report validation does not match bundle'}
}
function Assert-ReleaseLinkage($Link,$Manifest) {
    if ((Get-ReleaseField $Link 'target') -cne $script:LinuxTarget -or -not (Test-ReleaseEqual (Get-ReleaseField $Link 'artifact' object) $Manifest.artifact)) {throw 'Linkage artifact mismatch'}
    $checks=Get-ReleaseField $Link 'checks' object
    Assert-ReleaseNames $checks @('architecture','elf_class','interpreter','shared_libraries','glibc_versions') 'linkage checks'
    foreach ($property in $checks.PSObject.Properties) {Assert-ReleaseState $property.Value 'linkage check' -Required}
    $raw=Get-ReleaseField $Link 'readelf' object; $header=Get-ReleaseField $raw 'header'
    $elf=Get-ReleaseField $Link 'elf' object
    if ($header -cnotmatch '(?m)^\s*Class:\s+ELF64\s*$' -or $header -cnotmatch '(?m)^\s*Machine:\s+Advanced Micro Devices X86-64\s*$' -or
        $header -cnotmatch '(?m)^\s*Type:\s+(EXEC|DYN)\b' -or $elf.class -cne 'ELF64' -or $elf.architecture -cne 'x86_64' -or $elf.type -cne $Matches[1]) {throw 'Invalid linkage ELF header'}
    $program=Get-ReleaseField $raw 'program'; $dynamic=Get-ReleaseField $raw 'dynamic'; $versions=Get-ReleaseField $raw 'versions'
    if ($program -cmatch '(?m)^\s*INTERP\s|Requesting program interpreter:' -or $dynamic -cmatch '\(NEEDED\)' -or $versions -cmatch 'GLIBC_' -or $program -cnotmatch '(?m)^\s*LOAD\s') {throw 'Dynamic linkage evidence rejected'}
    $null=Get-ReleaseField $Link 'readelf_version'
}
function Assert-ReleaseRegression([string]$Directory,$Report,$Manifest) {
    foreach ($key in @('target','artifact','package_version','lockfile_sha256','rustc_version','cargo_version','build_image','native_tools')) {if (-not (Test-ReleaseEqual (Get-ReleaseField $Report $key $(if ($key -in @('artifact','native_tools')) {'object'} else {'string'})) $Manifest.$key)) {throw "Regression $key does not match bundle"}}
    if ((Get-ReleaseField $Report 'optional_telemetry' boolean) -or (Get-ReleaseField $Report 'reported_version') -cne ('mmhmem '+$Manifest.package_version)) {throw 'Regression feature/version mismatch'}
    $dependencies=Get-ReleaseField $Report 'active_dependencies' array
    if ($dependencies -cnotcontains 'mmhmem' -or $dependencies -cnotcontains 'rustls' -or @($dependencies | Where-Object {$_ -isnot [string] -or $_ -cmatch '^opentelemetry|^(native-tls|openssl|openssl-sys)$'}).Count) {throw 'Invalid plain target dependencies'}
    $checks=Get-ReleaseField $Report 'checks' object; Assert-ReleaseNames $checks @('target-dependencies','clippy','tests','release-build') 'regression checks'
    foreach ($property in $checks.PSObject.Properties) {
        Assert-ReleaseState $property.Value 'regression check' -Required
        $log=Get-ReleaseBundleFile $Directory (Get-ReleaseField $property.Value 'report') -Report
        if ([string]::IsNullOrWhiteSpace([IO.File]::ReadAllText($log))) {throw 'Regression diagnostic log is empty'}
    }
}
function Assert-ReleaseRuntime([string]$Directory,$Runtime,$Manifest,$Seen) {
    Assert-ReleaseState $Runtime 'runtime'
    Assert-ReleaseImage (Get-ReleaseField $Runtime 'image') $(if ($Runtime.name -ceq 'debian-bookworm') {'debian:bookworm-slim'} else {'alpine:3.22'})
    $null=Get-ReleaseField $Runtime 'kernel'; $null=Get-ReleaseField $Runtime 'cpu'
    if ((Get-ReleaseField $Runtime 'architecture') -cne 'x86_64' -or (Get-ReleaseField $Runtime 'artifact_sha256') -cne $Manifest.artifact.sha256) {throw 'Runtime artifact/architecture mismatch'}
    $cases=Get-ReleaseField $Runtime 'cases' object; Assert-ReleaseNames $cases $script:LinuxRuntimeCases 'runtime cases'
    foreach ($property in $cases.PSObject.Properties) {Assert-ReleaseState $property.Value 'runtime case' -Required}
    if ($cases.pristine_startup.artifact_sha256 -cne $Manifest.artifact.sha256 -or $cases.pristine_startup.reported_version -cne ('mmhmem '+$Manifest.package_version)) {throw 'Pristine startup identity/version does not match candidate'}
    $relative=Get-ReleaseField $Runtime 'report'
    if (-not $Seen.Add($relative)) {throw 'Duplicate report path'}
    $report=Read-ReleaseJson (Get-ReleaseBundleFile $Directory $relative -Report)
    Assert-ReleaseReportIdentity $report $Manifest -ValidationRequired
    foreach ($key in @('name','image','kernel','architecture','cpu','artifact_sha256','cases')) {if (-not (Test-ReleaseEqual $report.$key $Runtime.$key)) {throw "Runtime report $key does not match bundle"}}
}
function Assert-LinuxReleaseBundle([string]$Directory,[string]$Revision,[string]$ValidationId) {
    $root=Get-Item -LiteralPath $Directory -Force
    if (-not $root.PSIsContainer -or ($root.Attributes -band [IO.FileAttributes]::ReparsePoint)) {throw 'Bundle root must be an ordinary directory'}
    $manifest=Read-ReleaseJson (Get-ReleaseBundleFile $Directory 'verification.json')
    Assert-ReleaseState $manifest 'bundle'
    if ((Get-ReleaseField $manifest 'schema_version' integer) -ne 1) {throw 'Unsupported schema version'}
    if ((Get-ReleaseField $manifest 'revision') -cne $Revision -or $Revision -cnotmatch '^[0-9a-f]{40}$') {throw 'Bundle revision mismatch'}
    if ((Get-ReleaseField $manifest 'validation_id') -cne $ValidationId) {throw 'Bundle validation identity mismatch'}
    if ((Get-ReleaseField $manifest 'target') -cne $script:LinuxTarget) {throw 'Wrong bundle target'}
    if (-not (Get-ReleaseField $manifest 'source_clean' boolean) -or -not (Get-ReleaseField $manifest 'publishable' boolean)) {throw 'Bundle is not clean and publishable'}
    $null=Get-ReleaseField $manifest 'package_version'
    foreach ($tool in @('rustc','cargo')) {if ((Get-ReleaseField $manifest ($tool+'_version')) -cnotmatch ('^'+$tool+' 1\.98\.1(\s|$)')) {throw "Unapproved $tool version"}}
    if ((Get-ReleaseField $manifest 'lockfile_sha256') -cnotmatch '^[0-9a-f]{64}$') {throw 'Malformed lockfile hash'}
    Assert-ReleaseImage (Get-ReleaseField $manifest 'build_image') 'rust:1.98-slim-bookworm'
    $tools=Get-ReleaseField $manifest 'native_tools' object
    $null=Get-ReleaseField $tools 'musl-gcc'; $null=Get-ReleaseField $tools 'readelf'
    $artifact=Get-ReleaseField $manifest 'artifact' object
    $hash=Get-ReleaseField $artifact 'sha256'
    $binary=Get-ReleaseBundleFile $Directory $script:LinuxAssetName
    if ((Get-ReleaseField $artifact 'name') -cne $script:LinuxAssetName -or $hash -cnotmatch '^[0-9a-f]{64}$' -or
        (Get-ReleaseField $artifact 'size_bytes' integer) -le 0 -or (Get-Item $binary).Length -ne $artifact.size_bytes -or
        (Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash.ToLowerInvariant() -cne $hash) {throw 'Artifact checksum/size mismatch'}
    if ([IO.File]::ReadAllText((Get-ReleaseBundleFile $Directory 'SHA256SUMS')) -cne ($hash+"  $($script:LinuxAssetName)`n")) {throw 'Invalid SHA256SUMS'}
    $seen=New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::Ordinal)
    $scratch=Get-ReleaseField $manifest 'scratch' object
    Assert-ReleaseState $scratch 'scratch' -Required
    $relative=Get-ReleaseField $scratch 'report'; $null=$seen.Add($relative)
    $scratchReport=Read-ReleaseJson (Get-ReleaseBundleFile $Directory $relative -Report)
    Assert-ReleaseReportIdentity $scratchReport $manifest -ValidationRequired
    foreach ($key in @('status','validation_id','artifact_sha256','reported_version')) {
        if (-not (Test-ReleaseEqual $scratch.$key $scratchReport.$key)) {throw "Scratch report $key does not match bundle"}
    }
    if ($scratch.validation_id -cne $ValidationId -or $scratch.artifact_sha256 -cne $hash -or $scratch.reported_version -cne ('mmhmem '+$manifest.package_version)) {throw 'Scratch candidate identity/version mismatch'}
    $reports=@{}
    foreach ($kind in @('linkage','regression')) {
        $entry=Get-ReleaseField $manifest $kind object; Assert-ReleaseState $entry $kind -Required
        $relative=Get-ReleaseField $entry 'report'
        if (-not $seen.Add($relative)) {throw 'Duplicate report path'}
        $reports[$kind]=Read-ReleaseJson (Get-ReleaseBundleFile $Directory $relative -Report)
        Assert-ReleaseReportIdentity $reports[$kind] $manifest -ValidationRequired:($kind -eq 'regression')
    }
    Assert-ReleaseLinkage $reports.linkage $manifest
    Assert-ReleaseRegression $Directory $reports.regression $manifest
    $runtimes=Get-ReleaseField $manifest 'runtimes' array
    if ($runtimes.Count -ne 2 -or (@($runtimes.name | Sort-Object) -join ',') -cne 'alpine-3.22,debian-bookworm') {throw 'Missing or duplicate runtime records'}
    foreach ($runtime in $runtimes) {Assert-ReleaseRuntime $Directory $runtime $manifest $seen}
    return $manifest
}
