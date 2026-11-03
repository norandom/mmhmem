$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot '../deploy/manage_routing.ps1')
$root=Join-Path $PSScriptRoot ('../tmp/routing-cross-'+[guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($root)|Out-Null
$canonical=Join-Path $root 'canonical.md'
[IO.File]::WriteAllText($canonical,'Updated routing.',(New-Object Text.UTF8Encoding($false)))
function Assert-Bytes($Path,$Expected) {
 if ([Convert]::ToBase64String([IO.File]::ReadAllBytes($Path)) -cne [Convert]::ToBase64String($Expected)) {throw 'Cross-helper byte mismatch'}
}
function Python-Commit($Operation,$Path) {
 & py -3.12 (Join-Path $PSScriptRoot '../deploy/manage_routing.py') $Operation $Path --canonical $canonical | Out-Null
 if($LASTEXITCODE -ne 0){throw 'Python helper failed'}
}
$utf8=New-Object Text.UTF8Encoding($false)
$values=@($utf8.GetBytes("User Unicode $([char]0x03bb)`n"),$utf8.GetBytes("User`r`n"),$utf8.GetBytes('No final newline'),[byte[]](239,187,191,85,115,101,114))
foreach($original in $values){
 foreach($first in @('powershell','python')){
  $path=Join-Path $root ([guid]::NewGuid().ToString('N')+'.md')
  [IO.File]::WriteAllBytes($path,$original)
  if($first -eq 'powershell'){
   Invoke-RoutingCommit (New-RoutingPlan $path install $utf8.GetBytes('Initial routing.'))
   Python-Commit install $path
   Python-Commit remove $path
  }else{
   Python-Commit install $path
   Invoke-RoutingCommit (New-RoutingPlan $path install $utf8.GetBytes('Other routing.'))
   Invoke-RoutingCommit (New-RoutingPlan $path remove $null)
  }
  Assert-Bytes $path $original
 }
}
$path=Join-Path $root 'invalid.md'
Invoke-RoutingCommit (New-RoutingPlan $path install $utf8.GetBytes('Routing.'))
[IO.File]::AppendAllText($path,"<!-- mmhmem:routing:start -->",$utf8)
$invalid=[IO.File]::ReadAllBytes($path)
$refused=$false;try{Invoke-RoutingCommit (New-RoutingPlan $path remove $null)}catch{$refused=$true}
if(-not $refused){throw 'PowerShell accepted malformed cross output'}
$nativeError=$ErrorActionPreference;$ErrorActionPreference='Continue'
& py -3.12 (Join-Path $PSScriptRoot '../deploy/manage_routing.py') remove $path 2>$null | Out-Null
$code=$LASTEXITCODE;$ErrorActionPreference=$nativeError
if($code -eq 0){throw 'Python accepted malformed cross output'}
Assert-Bytes $path $invalid
Write-Host 'PASS: real helper cross-update/removal preserves all original bytes'
