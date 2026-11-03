$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '../deploy/manage_routing.ps1')
. (Join-Path $PSScriptRoot '../tests/fixtures/routing-instructions/runner.ps1')
$testRoot = Join-Path $PSScriptRoot '../tmp/routing-native-tests'
[IO.Directory]::CreateDirectory($testRoot) | Out-Null
$count = Invoke-RoutingGoldenCases -Adapter {
 param($case,$source,$canonical)
 $path = Join-Path $testRoot ($case.id + '.md')
 if ([IO.File]::Exists($path)) { [IO.File]::Delete($path) }
 if ($null -ne $source) { [IO.File]::WriteAllBytes($path,$source) }
 try { New-RoutingPlan -Path $path -Operation $case.operation -Canonical $canonical -Dedicated:$case.dedicated }
 catch { @{ Outcome='error'; Bytes=$null } }
}
Write-Host "PASS: $count golden plans"
function Assert-True { param($Value,[string]$Message) if (-not $Value) { throw $Message } }
function Assert-Throws { param([scriptblock]$Action) $failed=$false; try { & $Action } catch { $failed=$true }; Assert-True $failed 'Expected refusal' }
$path=Join-Path $testRoot 'commit.md'
$canonical=[Text.Encoding]::UTF8.GetBytes('Read via smart_read.')
[IO.File]::WriteAllText($path,'User text',[Text.Encoding]::UTF8)
$original=[IO.File]::ReadAllBytes($path)
$plan=New-RoutingPlan $path install $canonical
$backup=$path+'.mmhmem.bak'; if ([IO.File]::Exists($backup)) { [IO.File]::Delete($backup) }
Invoke-RoutingCommit $plan -DryRun
Assert-True (-not [IO.File]::Exists($backup)) 'Dry run created backup'
foreach ($failure in @('backup','stage','replace')) {
 Assert-Throws { Invoke-RoutingCommit $plan -FailAt $failure }
 Assert-True ([Convert]::ToBase64String([IO.File]::ReadAllBytes($path)) -ceq [Convert]::ToBase64String($original)) 'Failure changed original'
}
Invoke-RoutingCommit $plan
Assert-True ([Convert]::ToBase64String([IO.File]::ReadAllBytes($backup)) -ceq [Convert]::ToBase64String($original)) 'Backup differs'
$installed=[IO.File]::ReadAllBytes($path)
Invoke-RoutingCommit (New-RoutingPlan $path install $canonical)
Assert-True ([Convert]::ToBase64String([IO.File]::ReadAllBytes($backup)) -ceq [Convert]::ToBase64String($original)) 'Noop rotated backup'
Invoke-RoutingCommit (New-RoutingPlan $path remove $null)
Assert-True ([Convert]::ToBase64String([IO.File]::ReadAllBytes($path)) -ceq [Convert]::ToBase64String($original)) 'Removal differs'
Assert-True ([Convert]::ToBase64String([IO.File]::ReadAllBytes($backup)) -ceq [Convert]::ToBase64String($installed)) 'Backup not previous version'
$plan=New-RoutingPlan $path install $canonical
Assert-Throws { Invoke-RoutingCommit $plan -BeforeCommit { [IO.File]::WriteAllText($path,'Concurrent edit') } }
Assert-True ([IO.File]::ReadAllText($path) -ceq 'Concurrent edit') 'Concurrent edit lost'
$new=Join-Path $testRoot 'new-target.md'; if ([IO.File]::Exists($new)) { [IO.File]::Delete($new) }
$plan=New-RoutingPlan $new install $canonical
Assert-Throws { Invoke-RoutingCommit $plan -BeforeCommit { [IO.File]::WriteAllText($new,'Appeared') } }
Assert-True ([IO.File]::ReadAllText($new) -ceq 'Appeared') 'New target clobbered'
$missing=Join-Path $testRoot ('absent-'+[guid]::NewGuid().ToString('N')+'/file.md')
Invoke-RoutingCommit (New-RoutingPlan $missing install $canonical) -DryRun
Assert-True (-not [IO.Directory]::Exists([IO.Path]::GetDirectoryName($missing))) 'Dry run created directory'
$dedicated=Join-Path $testRoot 'dedicated.md'; if ([IO.File]::Exists($dedicated)) { [IO.File]::Delete($dedicated) }
Invoke-RoutingCommit (New-RoutingPlan $dedicated install $canonical -Dedicated)
Invoke-RoutingCommit (New-RoutingPlan $dedicated remove $null -Dedicated)
Assert-True (-not [IO.File]::Exists($dedicated)) 'Owned empty dedicated file retained'
$link=Join-Path $testRoot ('linked-'+[guid]::NewGuid().ToString('N'))
New-Item -ItemType Junction -Path $link -Target $testRoot | Out-Null
try { Assert-Throws { New-RoutingPlan (Join-Path $link 'commit.md') install $canonical } }
finally { [IO.Directory]::Delete($link) }
Write-Host 'PASS: commits, backups, failure preservation, concurrent edits, dry-run, removal and reparse refusal'
