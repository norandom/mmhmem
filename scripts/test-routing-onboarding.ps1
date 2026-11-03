$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot '../deploy/routing_onboarding.ps1')
$root=Join-Path (Split-Path $PSScriptRoot -Parent) ('tmp/routing-onboarding-'+[guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($root)|Out-Null
$project=Join-Path $root 'project'; [IO.Directory]::CreateDirectory($project)|Out-Null
$names=@('CLAUDE_CONFIG_DIR','OPENCODE_CONFIG_DIR','OPENCODE_CONFIG','OPENCODE_CONFIG_CONTENT','XDG_CONFIG_HOME','OPENCODE_DISABLE_PROJECT_CONFIG','CLAUDE_CODE_SIMPLE','CLAUDE_CODE_DISABLE_PROJECT_INSTRUCTIONS')
$saved=@{}; foreach($name in $names){$saved[$name]=[Environment]::GetEnvironmentVariable($name);[Environment]::SetEnvironmentVariable($name,$null)}
try {
 foreach($harness in @('claude','opencode')) {
  foreach($global in @($false,$true)) {
   $plan=New-HarnessRoutingPlan -Harness $harness -UserRoot $root -Project $project -Global:$global
   Invoke-RoutingCommit $plan -DryRun
   if(Test-Path $plan.Path){throw 'Dryrun wrote instructions'}
   Complete-HarnessRouting $plan
   $again=New-HarnessRoutingPlan -Harness $harness -UserRoot $root -Project $project -Global:$global
   if($again.Outcome -ne 'unchanged'){throw 'Repeat duplicated guidance'}
   $remove=New-HarnessRoutingPlan -Harness $harness -UserRoot $root -Project $project -Global:$global -Remove
   Complete-HarnessRouting $remove
  }
 }
 foreach($name in $names) {
  [Environment]::SetEnvironmentVariable($name,'secret-never-echo')
  $harness=if($name -like 'CLAUDE*'){'claude'}else{'opencode'}
  if($name -eq 'OPENCODE_DISABLE_PROJECT_CONFIG'){[Environment]::SetEnvironmentVariable($name,'1')}
  $failed=$false
  try {New-HarnessRoutingPlan -Harness $harness -UserRoot $root -Project $project|Out-Null}catch{$failed=$true;if($_.Exception.Message.Contains('secret-never-echo')){throw 'Secret echoed'}}
  if(-not $failed){throw "Override accepted: $name"}
  [Environment]::SetEnvironmentVariable($name,$null)
 }
 $claude=Join-Path $project '.claude/rules/mmhmem.md';[IO.File]::WriteAllText($claude,'unowned')
 $failed=$false;try{New-HarnessRoutingPlan -Harness claude -UserRoot $root -Project $project|Out-Null}catch{$failed=$true}
 if(-not $failed -or [IO.File]::ReadAllText($claude) -ne 'unowned'){throw 'Unowned instructions changed'}
 # Exercise real engine ordering with a disposable source copy and fake harness CLIs.
 $tree=Join-Path $root 'engine'; [IO.Directory]::CreateDirectory($tree)|Out-Null
 Copy-Item (Join-Path $PSScriptRoot '../deploy') $tree -Recurse
 $engine=Join-Path $tree 'deploy/install_local.ps1'
 $source=[IO.File]::ReadAllText($engine).Replace('$HOME','$RoutingTestRoot')
 $source=$source.Replace('$ErrorActionPreference = ''Stop''',('$ErrorActionPreference = ''Stop'''+"`n"+'$RoutingTestRoot = '+"'"+$root.Replace("'","''")+"'"))
 [IO.File]::WriteAllText($engine,$source,(New-Object Text.UTF8Encoding($false)))
 $fake=Join-Path $root 'mmhmem.exe';[IO.File]::WriteAllText($fake,'fixture')
 $cli=Join-Path $root 'claude.cmd';[IO.File]::WriteAllText($cli,'@exit /b 0')
 $oldPath=$env:PATH;$env:PATH=$root+';'+$oldPath
 [IO.Directory]::CreateDirectory((Join-Path $root '.config/opencode'))|Out-Null
 [IO.File]::WriteAllText((Join-Path $root '.config/opencode/opencode.json'),'{}')
 try {
  $shell=(Get-Process -Id $PID).Path
  $ErrorActionPreference='Continue'
  $output=& $shell -NoProfile -File $engine -ProjectDir $project -Binary $fake -Agent opencode -NonInteractive -DryRun 2>&1|Out-String
  # opencode detection is supplied through the default global config.
  if($LASTEXITCODE -ne 0){
   [IO.Directory]::CreateDirectory((Join-Path $root '.config/opencode'))|Out-Null
   [IO.File]::WriteAllText((Join-Path $root '.config/opencode/opencode.json'),'{}')
   $ErrorActionPreference='Continue'
  $output=& $shell -NoProfile -File $engine -ProjectDir $project -Binary $fake -Agent opencode -NonInteractive -DryRun 2>&1|Out-String
  }
  if($LASTEXITCODE -ne 0 -or $output -notmatch 'routing instructions' -or $output -notmatch 'fresh harness session'){throw "Engine dryrun missed guidance: $output"}
  $bad=Join-Path $project 'AGENTS.md';[IO.File]::WriteAllText($bad,'<!-- mmhmem:routing:start -->')
  $ErrorActionPreference='Continue'
  $output=& $shell -NoProfile -File $engine -ProjectDir $project -Binary $fake -Agent opencode -NonInteractive 2>&1|Out-String
  if($LASTEXITCODE -eq 0 -or (Test-Path (Join-Path $project 'opencode.json'))){throw 'Failed preflight allowed registration write'}
 } finally {$env:PATH=$oldPath}
 Write-Host 'PASS: scope, repeat, remove, dry-run, overrides, ownership and engine preflight ordering'

} finally {foreach($name in $names){[Environment]::SetEnvironmentVariable($name,$saved[$name])}}
