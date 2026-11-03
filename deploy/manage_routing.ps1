# Dot-source for byte-preserving routing instruction plans and commits.
function Assert-RoutingPath {
 param([string]$Path)
 $current = [IO.Path]::GetFullPath($Path)
 while ($current) {
  if (Test-Path -LiteralPath $current) {
   if ((Get-Item -LiteralPath $current -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Reparse routing path: $current" }
  }
  $parent = [IO.Path]::GetDirectoryName($current)
  if ($parent -eq $current) { break }; $current = $parent
 }
}
function Get-RoutingText {
 param([byte[]]$Bytes)
 $utf8 = New-Object Text.UTF8Encoding($false,$true)
 return $utf8.GetString($Bytes)
}
function Get-RoutingSection {
 param([string]$Text)
 $start = '<!-- mmhmem:routing:start -->'; $end = '<!-- mmhmem:routing:end -->'
 $matches = [regex]::Matches($Text,'(?m)^.*mmhmem:routing:.*(?:\r?\n|$)')
 $first = $null; $last = $null; $prefix = 'none'; $metadata = $false
 foreach ($m in $matches) {
  $line = $m.Value.TrimEnd([char[]]"`r`n")
  if ($line -ceq $start -and $null -eq $first -and $null -eq $last) { $first=$m; continue }
  if ($line -ceq $end -and $null -ne $first -and $null -eq $last) { $last=$m; continue }
  if ($line -cmatch '^<!-- mmhmem:routing:prefix=(none|lf|crlf) -->$' -and $null -ne $first -and $null -eq $last -and -not $metadata) {
   $prefix=$Matches[1]; $metadata=$true; continue
  }
  throw 'Ambiguous routing markers or prefix metadata; file unchanged.'
 }
 if (($null -eq $first) -ne ($null -eq $last)) { throw 'Unmatched routing marker; file unchanged.' }
 if ($null -eq $first) { return $null }
 $from=$first.Index; $to=$last.Index+$last.Length
 $owned = if ($prefix -eq 'lf') { "`n" } elseif ($prefix -eq 'crlf') { "`r`n" } else { '' }
 if ($owned) {
  if ($from -lt $owned.Length -or $Text.Substring($from-$owned.Length,$owned.Length) -cne $owned) { throw 'Routing prefix does not match preceding bytes.' }
  if ($prefix -eq 'lf' -and $from -ge 2 -and $Text[$from-2] -eq "`r") { throw 'Routing LF prefix conflicts with CRLF.' }
  $from -= $owned.Length
 }
 return @{ From=$from; To=$to; Prefix=$prefix }
}
function New-RoutingPlan {
 param([string]$Path,[ValidateSet('install','remove')][string]$Operation,[byte[]]$Canonical,[switch]$Dedicated)
 Assert-RoutingPath $Path
 $exists=[IO.File]::Exists($Path); $source=$null
 if ($exists) { $source=[IO.File]::ReadAllBytes($Path) }
 $text=if ($exists) { Get-RoutingText $source } else { '' }
 $bom=''; if ($text.Length -gt 0 -and [int]$text[0] -eq 0xfeff) { $bom=[string][char]0xfeff; $text=$text.Substring(1) }
 if ($Dedicated -and $text -match '^---(?:\r?\n|$)') { throw 'Dedicated routing frontmatter needs manual review.' }
 $section=Get-RoutingSection $text
 if ($Dedicated -and $exists -and $text.Length -gt 0 -and $null -eq $section) { throw 'Unowned dedicated routing file; file unchanged.' }
 $newline=if ($text.Contains("`r`n")) { "`r`n" } else { "`n" }
 $result=$text
 if ($Operation -eq 'remove') {
  if ($null -ne $section) { $result=$text.Substring(0,$section.From)+$text.Substring($section.To) }
 } else {
  $body=(Get-RoutingText $Canonical).TrimEnd([char[]]"`r`n").Replace("`r`n","`n").Replace("`n",$newline)
  if ($body.Contains('mmhmem:routing:')) { throw 'Canonical instructions contain ownership markers.' }
  $prefix='none'; $separator=''
  if ($null -ne $section) { $prefix=$section.Prefix }
  elseif ($text.Length -gt 0 -and -not $text.EndsWith("`n")) { $separator=$newline; $prefix=if ($newline -eq "`r`n") { 'crlf' } else { 'lf' } }
  if ($null -ne $section -and $prefix -ne 'none') { $separator=if ($prefix -eq 'crlf') { "`r`n" } else { "`n" } }
  $block=$separator+'<!-- mmhmem:routing:start -->'+$newline+"<!-- mmhmem:routing:prefix=$prefix -->"+$newline+$body+$newline+'<!-- mmhmem:routing:end -->'+$newline
  if ($null -ne $section) { $result=$text.Substring(0,$section.From)+$block+$text.Substring($section.To) } else { $result=$text+$block }
 }
 $bytes=$null; $outcome='changed'
 if ($Operation -eq 'remove' -and -not $exists) { $outcome='unchanged' }
 elseif ($Operation -eq 'remove' -and $Dedicated -and $result.Length -eq 0 -and $null -ne $section) { $outcome='delete' }
 else {
  $bytes=(New-Object Text.UTF8Encoding($false)).GetBytes($bom+$result)
  if ($exists -and [Convert]::ToBase64String($source) -ceq [Convert]::ToBase64String($bytes)) { $outcome='unchanged' }
 }
 return @{ Path=[IO.Path]::GetFullPath($Path); Exists=$exists; Original=$source; Outcome=$outcome; Bytes=$bytes }
}
function Invoke-RoutingCommit {
 param([Parameter(Mandatory=$true)]$Plan,[switch]$DryRun,[scriptblock]$BeforeCommit,[ValidateSet('','backup','stage','replace')][string]$FailAt='')
 if ($DryRun -or $Plan.Outcome -eq 'unchanged') { return }
 $path=$Plan.Path; Assert-RoutingPath $path
 if ($BeforeCommit) { & $BeforeCommit }
 Assert-RoutingPath $path
 $exists=[IO.File]::Exists($path)
 if ($exists -ne $Plan.Exists) { throw 'Routing target changed after preflight.' }
 if ($exists -and [Convert]::ToBase64String([IO.File]::ReadAllBytes($path)) -cne [Convert]::ToBase64String($Plan.Original)) { throw 'Routing content changed after preflight.' }
 $directory=[IO.Path]::GetDirectoryName($path)
 [IO.Directory]::CreateDirectory($directory) | Out-Null
 $stage=Join-Path $directory ('.mmhmem-'+[guid]::NewGuid().ToString('N')+'.tmp')
 try {
  if ($FailAt -eq 'backup') { throw 'Injected backup failure' }
  if ($exists) {
   Assert-RoutingPath ($path+'.mmhmem.bak')
   [IO.File]::WriteAllBytes($stage,$Plan.Original)
   if ([IO.File]::Exists($path+'.mmhmem.bak')) { [IO.File]::Replace($stage,$path+'.mmhmem.bak',[NullString]::Value) } else { [IO.File]::Move($stage,$path+'.mmhmem.bak') }
  }
  if ($FailAt -eq 'stage') { throw 'Injected stage failure' }
  if ($Plan.Outcome -ne 'delete') { [IO.File]::WriteAllBytes($stage,$Plan.Bytes) }
  if ($FailAt -eq 'replace') { throw 'Injected replace failure' }
  Assert-RoutingPath $path
  if ([IO.File]::Exists($path) -ne $exists) { throw 'Routing target changed before commit.' }
  if ($exists -and [Convert]::ToBase64String([IO.File]::ReadAllBytes($path)) -cne [Convert]::ToBase64String($Plan.Original)) { throw 'Routing content changed before commit.' }
  if ($Plan.Outcome -eq 'delete') { [IO.File]::Delete($path) }
  elseif ($exists) { [IO.File]::Replace($stage,$path,[NullString]::Value) }
  else { [IO.File]::Move($stage,$path) }
 } finally { if ([IO.File]::Exists($stage)) { [IO.File]::Delete($stage) } }
}
