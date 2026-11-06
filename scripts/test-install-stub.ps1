# Offline checks of the root install.ps1 stub (reliable-installers task 9.2).
# The stub must print the README's Windows one-liner, ignore old options, download
# nothing, write nothing, exit 2 when run as a file and only set $LASTEXITCODE (never
# exit) when piped into iex. Every run below happens in an empty scratch folder.
param([string]$Stub, [string]$Scratch)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
if (-not $Stub) { $Stub = Join-Path $repo 'install.ps1' }
$ownScratch = -not $Scratch
if ($ownScratch) { $Scratch = Join-Path $repo ('tmp/install-stub-' + [Guid]::NewGuid().ToString('N')) }
New-Item -ItemType Directory -Force -Path $Scratch | Out-Null
$oneLiner = 'irm https://github.com/norandom/mmhmem/releases/latest/download/install.ps1 | iex'
$readme = [IO.File]::ReadAllLines((Join-Path $repo 'README.md'))
if (-not ($readme -ccontains $oneLiner)) { throw 'README.md does not carry the Windows one-liner the stub prints' }
$text = [IO.File]::ReadAllText($Stub)
# No network or file-writing command apart from the printed one-liner itself.
$code = $text.Replace($oneLiner, '')
foreach ($word in @('Invoke-WebRequest','Invoke-RestMethod','iwr','irm','curl','wget','WebClient','HttpClient','Start-BitsTransfer','Set-Content','Out-File','New-Item','Copy-Item','Move-Item','WriteAll','Start-Process','setx','Environment]::Set')) {
    if ($code -match ('(?i)(^|[^A-Za-z-])' + [regex]::Escape($word) + '([^A-Za-z-]|$)')) { throw "install.ps1 stub uses $word" }
}
function Get-Snapshot([string]$Dir) {
    @(Get-ChildItem -LiteralPath $Dir -Recurse -Force | ForEach-Object { '{0} {1} {2}' -f $_.FullName, $_.Length, $_.LastWriteTimeUtc.Ticks }) -join "`n"
}
function Assert-Prints([string]$Output, [string]$What) {
    $lines = @($Output -split "`r?`n" | ForEach-Object { $_.Trim() })
    if (-not ($lines -ccontains $oneLiner)) { throw "${What}: the one-liner is missing: $Output" }
    foreach ($part in @('has moved', 'did not download or change anything', 'latest signed release', 'mmhmem setup', 'mmhmem update')) {
        if (-not $Output.Contains($part)) { throw "${What}: '$part' is missing: $Output" }
    }
}
$work = Join-Path $Scratch 'work'
New-Item -ItemType Directory -Force -Path $work | Out-Null
$copy = Join-Path $work 'install.ps1'
Copy-Item -LiteralPath $Stub -Destination $copy
$before = Get-Snapshot $work
$shells = @()
foreach ($name in @('pwsh', 'powershell')) {
    $cmd = Get-Command $name -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($cmd) { $shells += $cmd.Source }
}
if (-not $shells.Count) { throw 'No PowerShell executable found' }
$quoted = $copy.Replace("'", "''")
foreach ($shell in $shells) {
    Push-Location $work
    try {
        # Saved and run as a file, with old options: they are ignored, exit code 2.
        $out = & $shell -NoProfile -NonInteractive -File $copy -Agent codex -Hooks claude -Remove -DryRun -WorkerModel m 2>&1 | Out-String
        if ($LASTEXITCODE -ne 2) { throw "$shell -File: exit code $LASTEXITCODE, expected 2" }
        Assert-Prints $out "$shell -File"
        # Piped into iex: no exit (the next statement still runs), $LASTEXITCODE is 2,
        # and the helper function does not stay behind in the session.
        $line = "`$t = [IO.File]::ReadAllText('$quoted'); `$t | iex; Write-Output ('AFTER=' + `$LASTEXITCODE + ' FN=' + [bool](Get-Command Show-MmhmemMoved -ErrorAction SilentlyContinue))"
        $out = & $shell -NoProfile -NonInteractive -Command $line 2>&1 | Out-String
        if ($LASTEXITCODE -ne 0) { throw "$shell iex: the session ended with $LASTEXITCODE (the stub must not exit)" }
        if (-not $out.Contains('AFTER=2 FN=False')) { throw "$shell iex: wrong state after the stub: $out" }
        Assert-Prints $out "$shell iex"
    } finally { Pop-Location }
}
# In this process, with download commands replaced: none of them is called.
$script:Calls = 0
function Invoke-WebRequest { $script:Calls++ }
function Invoke-RestMethod { $script:Calls++ }
$global:LASTEXITCODE = 0
$out = & $copy -Agent codex 6>&1 | Out-String
if ($LASTEXITCODE -ne 2 -or $script:Calls -ne 0) { throw "In-process run: exit $LASTEXITCODE, downloads $($script:Calls)" }
Assert-Prints $out 'in-process run'
if ((Get-Snapshot $work) -cne $before) { throw 'The stub changed files in its folder' }
if (@(Get-ChildItem -LiteralPath $work -Force).Count -ne 1) { throw 'The stub left files behind' }
if ($ownScratch) { Remove-Item -LiteralPath $Scratch -Recurse -Force }
$global:LASTEXITCODE = 0
Write-Host "PASS: install.ps1 stub prints the release one-liner, ignores options, exit 2 as a file, no exit under iex, no downloads or writes ($($shells.Count) shell(s))"
