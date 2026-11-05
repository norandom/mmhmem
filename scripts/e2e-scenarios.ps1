#Requires -Version 5.1
<#
.SYNOPSIS
The installer end-to-end scenarios (reliable-installers task 8.2,
Requirement 10.3 and 10.4), dot-sourced by scripts/e2e-run.ps1.
.DESCRIPTION
Each Invoke-*Phase here is one CI step and checks its own result; a phase
that returns has passed. Every scenario takes a home snapshot first and
compares against it (e2e-home-state.ps1 has the rules). The menu runs use
the pseudo-terminal driver ci/e2e/pty-driver (Unix pty / Windows ConPTY):
it reads the screen and presses keys like a user, so the real full-screen
menu is driven, and its exit code is the program's.

  pty-selftest     the driver types into a shell prompt and reads the answer
  cancel-at-review one-liner in a terminal, Install -> agents -> features ->
                   Review, q: exit 2, "Nothing was changed.", home unchanged
  download-404     one-liner rendered for a folder without the archives:
                   non-zero, plain message, home byte-identical (strict)
  bad-hash         one-liner whose built-in SHA-256 is wrong: same
  install          (e2e-run.ps1) the one-liner without a terminal
  rerun            the one-liner again without a terminal: exit 0,
                   "Everything is current", home unchanged
  remove           `mmhmem setup` in a terminal, Remove: program, record and
                   PATH entry gone, nothing of mmhmem still running, seeded
                   files byte-identical, memories kept (same content)
  menu-install     the one-liner in a terminal through every menu screen,
                   all detected agents, confirm (checked by assert-installed)
  install-old      the previous release's one-liner without a terminal
  bad-signature    `mmhmem setup` offered a newer release signed by another
                   key: the update is refused, home unchanged, exit non-zero
  update           `mmhmem setup` updates to this commit's release: new
                   program hash recorded, choices kept, checks pass again
  invalid-config   an invalid opencode.json: OpenCode skipped with a plain
                   message and left byte-identical, the others installed
#>

# The release this platform downloads (archive key in package-release-archives.ps1).
function Get-PlatformKey {
    if ($script:OnWindows) {return 'windows'}
    if (-not $script:OnMac) {return 'linux'}
    $cpu=(& uname -m)
    if ($cpu -ceq 'arm64' -or $cpu -ceq 'aarch64') {return 'macos-aarch64'}
    return 'macos-x86_64'
}
function Get-ReleaseInfo([string]$Role='current') {
    $file=if ($Role -ceq 'current') {'release.json'} else {"release-$Role.json"}
    return Read-Json (Join-Path $script:Site $file)
}
# Escapes a release tag for the driver's regular expressions.
function ConvertTo-TagPattern([string]$Tag) {$Tag.Replace('.','\.')}
function Get-InstalledPaths {
    $rel=Get-MmhmemRelativePaths
    return @{Record=(Join-Path $script:SeedHome $rel.Record);Program=(Join-Path $script:SeedHome $rel.Program)
        DataDir=(Join-Path $script:SeedHome $rel.DataDir);Rel=$rel}
}
function Get-PtyDriver {
    $candidates=@($env:MMHMEM_E2E_PTY,(Join-Path $WorkDir ('pty/mmhmem-e2e-pty'+$(if ($script:OnWindows) {'.exe'} else {''}))))
    foreach ($c in $candidates) {if ($c -and (Test-Path -LiteralPath $c -PathType Leaf)) {return (Resolve-Path -LiteralPath $c).ProviderPath}}
    throw 'The pseudo-terminal driver is missing (build ci/e2e/pty-driver; MMHMEM_E2E_PTY names it)'
}
function Assert-NoFailures([string]$Phase,$Failures) {
    $list=@($Failures | Where-Object {$_})
    if ($list.Count) {foreach ($f in $list) {Write-Host "FAIL: $f"};throw "${Phase}: $($list.Count) failure(s)"}
    Write-Host "${Phase}: OK"
}
# A run's output. On Windows a program an agent CLI leaves running keeps the
# log open for writing: `opencode mcp list` starts OpenCode's own background
# service (`opencode serve --service`), which stays up by design and
# inherits setup's output handle (research.md section 8). ReadAllText would
# refuse the file; it is read sharing it with that writer, as Get-Content does.
function Read-Log([string]$Name) {
    $text=''
    foreach ($ext in @('out','err')) {
        $file=Join-Path $script:Logs "$Name.$ext"
        if (-not (Test-Path -LiteralPath $file)) {continue}
        $stream=New-Object IO.FileStream($file,[IO.FileMode]::Open,[IO.FileAccess]::Read,([IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete))
        try {$text+=(New-Object IO.StreamReader($stream,$true)).ReadToEnd()+"`n"} finally {$stream.Dispose()}
    }
    return $text
}

# ---------------------------------------------------------- one-liners ---
# This step's shell (powershell.exe 5.1 or pwsh). A Microsoft Store pwsh
# lives under WindowsApps, which only its app alias may start.
function Get-ShellPath {
    $path=(Get-Process -Id $PID).Path
    if ($path -like '*\WindowsApps\*') {
        $alias=@(Get-Command (Split-Path -Leaf $path) -CommandType Application -ErrorAction SilentlyContinue | Where-Object {$_.Source -notlike '*\Program Files\WindowsApps\*'})
        if ($alias.Count) {return $alias[0].Source}
    }
    return $path
}
# The one-liner without a terminal, from any rendered folder of the site.
function Invoke-OneLinerNoTerminal([string]$Name,[string]$BaseUrl) {
    $command=Get-OneLiner $BaseUrl
    Write-Host "Running the one-liner without a terminal: $($command.Arguments)"
    return (Invoke-Isolated $Name $command.File $command.Arguments)
}
# Program and arguments of the one-liner for the pseudo-terminal: what a
# user types at a shell prompt (Windows: plus `exit $LASTEXITCODE` so the
# harness sees setup's exit code; install.ps1 itself never calls exit).
function Get-OneLinerProgram([string]$BaseUrl) {
    if ($script:OnWindows) {
        return @{Program=(Get-ShellPath);Args=@('-NoProfile','-Command',"irm '$BaseUrl/install.ps1' | iex; exit `$LASTEXITCODE")}
    }
    return @{Program='/bin/sh';Args=@('-c',"curl -fsSL '$BaseUrl/install.sh' | sh")}
}
# Runs the driver on a script made of the program, its arguments and the
# steps, in the isolated environment. Writes logs/<Name>.exit (the
# program's exit code, 'killed' when the driver stopped it) and
# logs/<Name>.out (the last screen). Returns the driver's own exit code.
function Invoke-Pty([string]$Name,[string]$Program,[string[]]$Arguments,[string[]]$Steps) {
    $lines=@("program $Program")+@($Arguments | ForEach-Object {"arg $_"})+$Steps
    $scriptFile=Join-Path $script:Logs "$Name.pty.txt"
    Write-Utf8 $scriptFile (($lines -join "`n")+"`n")
    $prefix=Join-Path $script:Logs "$Name.pty"
    $driver=Get-PtyDriver
    $code=Invoke-Isolated "$Name.driver" $driver "`"$scriptFile`" `"$prefix`""
    $exit=if (Test-Path -LiteralPath "$prefix.exit") {([IO.File]::ReadAllText("$prefix.exit")).Trim()} else {'killed'}
    Write-Utf8 (Join-Path $script:Logs "$Name.exit") $exit
    if (Test-Path -LiteralPath "$prefix.final") {Copy-Item -LiteralPath "$prefix.final" -Destination (Join-Path $script:Logs "$Name.out") -Force}
    if ($code -ne 0) {
        if (Test-Path -LiteralPath "$prefix.screen") {Write-Host "----- $Name screens -----";Get-Content -LiteralPath "$prefix.screen" -Encoding UTF8 | Select-Object -Last 120 | ForEach-Object {Write-Host $_}}
        throw "${Name}: the menu did not go as expected (driver exit $code, program exit $exit); see logs/$Name.pty.screen"
    }
    return $code
}

# -------------------------------------------------------- pty-selftest ---
function Invoke-PtySelfTestPhase {
    if ($script:OnWindows) {
        $program=Get-ShellPath
        $arguments=@('-NoProfile','-Command',"`$n = Read-Host 'Your name'; Write-Host ""hello [`$n]""; exit 7")
    } else {
        $program='/bin/sh'
        $arguments=@('-c','printf "Your name: "; read n; echo "hello [$n]"; exit 7')
    }
    $null=Invoke-Pty 'pty-selftest' $program $arguments @('timeout 60','expect Your name','send text:Ada enter','expect hello \[Ada\]','exit 7')
    Write-Host 'pty-selftest: the pseudo-terminal types, reads and passes the exit code'
}

# --------------------------------------------------- cancel-at-review ---
function Invoke-CancelAtReviewPhase {
    Start-Server
    $before=Get-HomeState 'before-cancel-at-review'
    $one=Get-OneLinerProgram (Get-ReleaseInfo).base_url
    $null=Invoke-Pty 'cancel-at-review' $one.Program $one.Args @(
        'timeout 240','expect Install or update','send enter',
        'expect Which tools should mmhmem work with','send enter',
        'expect Which features do you want','send enter',
        'expect Review: this is what will change','send q',
        'timeout 60','exit 2','expect Nothing was changed\.')
    $failures=@(Compare-HomeState $before (Get-HomeState 'after-cancel-at-review'))
    $failures+=@(Get-BootstrapLeftovers | ForEach-Object {"the bootstrap left $_"})
    Assert-NoFailures 'cancel-at-review' $failures
}

# ------------------------------------------- download-404 and bad-hash ---
# A copy of the release's bootstraps rendered for <site>/<Folder>, with
# the archives (bad-hash) or without them (download-404).
function New-BrokenRelease([string]$Folder,[switch]$WithArchives,[switch]$WrongHash) {
    $release=Get-ReleaseInfo
    $source=Join-Path $script:Site $release.tag
    $target=Join-Path $script:Site $Folder
    if (Test-Path -LiteralPath $target) {Remove-Item -LiteralPath $target -Recurse -Force}
    New-Item -ItemType Directory -Path $target | Out-Null
    $baseUrl="http://127.0.0.1:$($release.port)/$Folder"
    $null=Write-RenderedBootstraps $release.tag $source $target $baseUrl
    if ($WithArchives) {
        foreach ($t in (Get-ReleaseArchiveTargets $release.tag)) {Copy-Item -LiteralPath (Join-Path $source $t.Archive) -Destination $target}
    }
    if ($WrongHash) {
        foreach ($name in @('install.sh','install.ps1')) {
            $path=Join-Path $target $name
            $text=[IO.File]::ReadAllText($path)
            foreach ($t in (Get-ReleaseArchiveTargets $release.tag)) {
                $hash=(Get-FileHash -LiteralPath (Join-Path $source $t.Archive) -Algorithm SHA256).Hash.ToLowerInvariant()
                $wrong=$(if ($hash[0] -ceq '0') {'1'} else {'0'})+$hash.Substring(1)
                $text=$text.Replace($hash,$wrong)
            }
            Write-Utf8 $path $text
        }
    }
    return $baseUrl
}
function Invoke-FailedDownloadScenario([string]$Name,[string]$BaseUrl,[string]$Message) {
    Start-Server
    $before=Get-HomeState "before-$Name"
    $code=Invoke-OneLinerNoTerminal $Name $BaseUrl
    $failures=@(Compare-HomeState $before (Get-HomeState "after-$Name") -Strict)
    if ($code -eq 0) {$failures+='the one-liner exited with 0'}
    $log=Read-Log $Name
    foreach ($want in @($Message,'Nothing was installed or changed on this computer.')) {
        if (-not $log.Contains($want)) {$failures+="the output does not say: $want"}
    }
    $failures+=@(Get-BootstrapLeftovers | ForEach-Object {"the bootstrap left $_"})
    Assert-NoFailures $Name $failures
}
function Invoke-Download404Phase {
    $url=New-BrokenRelease 'e2e-missing-archives'
    Invoke-FailedDownloadScenario 'download-404' $url 'The download failed:'
}
function Invoke-BadHashPhase {
    $url=New-BrokenRelease 'e2e-wrong-hash' -WithArchives -WrongHash
    Invoke-FailedDownloadScenario 'bad-hash' $url 'does not match the checksum built into this installer'
}

# --------------------------------------------------------------- rerun ---
function Invoke-RerunPhase {
    Start-Server
    $paths=Get-InstalledPaths
    if (-not (Test-Path -LiteralPath $paths.Record)) {throw 'rerun needs an install (run install first)'}
    $before=Get-HomeState 'before-rerun'
    $code=Invoke-OneLinerNoTerminal 'rerun' (Get-ReleaseInfo).base_url
    $failures=@(Compare-HomeState $before (Get-HomeState 'after-rerun'))
    if ($code -ne 0) {$failures+="the second run exited with $code"}
    if (-not (Read-Log 'rerun').Contains('Everything is current')) {$failures+='the second run did not say "Everything is current"'}
    $failures+=@(Wait-NoLeftoverProcesses | ForEach-Object {"still running after setup finished: $_"})
    $failures+=@(Get-BootstrapLeftovers | ForEach-Object {"the bootstrap left $_"})
    Assert-NoFailures 'rerun' $failures
}

# -------------------------------------------------------------- remove ---
# The memory store as "kept" means it: each SQLite database by its content
# (schema and every row, scripts/e2e-sqlite-digest.py, read from a copy),
# whatever of it still sits in the -wal file. The -wal and -shm files are
# SQLite's own journal and index of that database, so they are not compared
# on their own: which of them exist, and their bytes, change whenever a
# program opens or closes the store. Every other file byte for byte.
function Get-StoreDigest([string]$Database) {
    $python=Get-Python
    $out=& $python -I (Join-Path $PSScriptRoot 'e2e-sqlite-digest.py') $Database
    if ($LASTEXITCODE -ne 0 -or -not $out) {throw "could not read the memory store $Database"}
    return ([string]$out | ConvertFrom-Json)
}
function Get-StoreFiles([string]$Dir) {
    $map=[ordered]@{}
    if (-not (Test-Path -LiteralPath $Dir)) {return $map}
    foreach ($rel in (Get-SeedFiles $Dir)) {
        if ($rel -ceq 'install.json' -or $rel -clike 'backups/*') {continue}   # macOS: record folder = store folder
        if ($rel -clike '*.db-wal' -or $rel -clike '*.db-shm') {continue}
        $file=Join-Path $Dir $rel
        if ($rel -clike '*.db') {
            $d=Get-StoreDigest $file
            $rows=@($d.rows.PSObject.Properties | Where-Object {$_.Value} | ForEach-Object {"$($_.Name)=$($_.Value)"}) -join ' '
            Write-Host "  store $rel content $($d.sha256.Substring(0,16)) rows: $rows"
            $map[$rel]="content:$($d.sha256)"
            continue
        }
        $map[$rel]=(Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    return $map
}
function Invoke-RemovePhase {
    $paths=Get-InstalledPaths
    if (-not (Test-Path -LiteralPath $paths.Program)) {throw 'remove needs an install (run install first)'}
    # Nothing mmhmem started may still run: it would hold the store and the
    # program (research.md section 8).
    $left=Wait-NoLeftoverProcesses
    if ($left.Count) {Assert-NoFailures 'remove' @($left | ForEach-Object {"still running before the removal: $_"})}
    $store=Get-StoreFiles $paths.DataDir
    if (-not $store.Contains('memory.db')) {throw "no memory store at $($paths.DataDir) before the removal (the check should have created memory.db)"}
    $null=Get-HomeState 'before-remove'   # for the logs
    $null=Invoke-Pty 'remove' $paths.Program @('setup','--release-dir',(Join-Path $script:Site (Get-ReleaseInfo).tag)) @(
        'timeout 240','expect Newest release: v','select >\s+Remove mmhmem','send enter',
        'expect Also delete my saved memories','send enter',
        'timeout 600','expect enter or q close','send enter','timeout 60','exit 0')
    $failures=New-Object Collections.Generic.List[string]
    # Windows deletes the running program right after setup closed: it was
    # moved into %TEMP% (.mmhmem.<random>.__relocated__.exe, self_replace)
    # and a short-lived helper copy (.__selfdelete__.exe) deletes it there.
    $folder=Split-Path -Parent $paths.Program
    $temp=Join-Path $script:SeedHome 'AppData\Local\Temp'
    for ($i=0;$i -lt 120;$i++) {
        $inFolder=(Test-Path -LiteralPath $folder) -and @(Get-ChildItem -LiteralPath $folder -Force -File).Count
        $moved=$script:OnWindows -and (Test-Path -LiteralPath $temp) -and @(Get-ChildItem -LiteralPath $temp -Force -File -Filter '.mmhmem.*' | Where-Object {$_.Name -like '*.__relocated__.exe' -or $_.Name -like '*.__selfdelete__.exe'}).Count
        if (-not $inFolder -and -not $moved) {break}
        Start-Sleep -Milliseconds 500
    }
    if (Test-Path -LiteralPath $paths.Program) {$failures.Add('the program is still there')}
    if ($script:OnWindows -and (Test-Path -LiteralPath $temp)) {
        foreach ($f in @(Get-ChildItem -LiteralPath $temp -Force -File -Filter '.mmhmem.*' | Where-Object {$_.Name -like '*.__relocated__.exe'})) {$failures.Add("the program was moved to $($f.FullName) but not deleted")}
    }
    if (Test-Path -LiteralPath $paths.Record) {$failures.Add('the install record is still there')}
    foreach ($p in (Wait-NoLeftoverProcesses)) {$failures.Add("still running after the removal: $p")}
    $after=Get-HomeState 'after-remove'
    $seeded=Read-HomeState 'seeded'
    # Against the seed: everything mmhmem added is gone except its kept backups
    # and the memory store; the PATH is the one before the job.
    $kept=@("$($paths.Rel.RecordDir)/backups/*","$($paths.Rel.DataDir)/*")
    foreach ($f in (Compare-HomeState $seeded $after $kept)) {$failures.Add($f)}
    $now=Get-StoreFiles $paths.DataDir
    foreach ($rel in $store.Keys) {
        if ($now[$rel] -cne $store[$rel]) {$failures.Add("the memory store file $rel was not kept as it was$(if ($rel -clike '*.db') {' (its content, compared row by row)'})")}
    }
    Assert-NoFailures 'remove' $failures
}

# -------------------------------------------------------- menu-install ---
function Invoke-MenuInstallPhase {
    Start-Server
    $one=Get-OneLinerProgram (Get-ReleaseInfo).base_url
    $null=Invoke-Pty 'menu-install' $one.Program $one.Args @(
        'timeout 240','expect Install or update','send enter',
        'expect Which tools should mmhmem work with',
        'expect \[x\] Claude Code','expect \[x\] Codex','expect \[x\] OpenCode','send enter',
        'expect Which features do you want','send enter',
        'expect Review: this is what will change','send enter',
        'timeout 900','expect enter or q close','send enter','timeout 60','exit 0')
    Write-Host 'menu-install: confirmed through every screen; assert-installed -Run menu-install checks the result'
}

# --------------------------------------------- install-old, update etc. ---
function Get-AgentChoices($Record) {
    $choices=[ordered]@{}
    foreach ($p in $Record.agents.PSObject.Properties) {
        $on=@($p.Value.features.PSObject.Properties | Where-Object {$_.Value} | ForEach-Object {$_.Name} | Sort-Object)
        if ($on.Count) {$choices[$p.Name]=$on -join ','}
    }
    return $choices
}
function Invoke-InstallOldPhase {
    Start-Server
    $old=Get-ReleaseInfo 'old'
    $code=Invoke-OneLinerNoTerminal 'install-old' $old.base_url
    $paths=Get-InstalledPaths
    $failures=New-Object Collections.Generic.List[string]
    if ($code -ne 0) {$failures.Add("the previous release's one-liner exited with $code")}
    if (-not (Test-Path -LiteralPath $paths.Record)) {$failures.Add('no install record');Assert-NoFailures 'install-old' $failures}
    $record=Read-Json $paths.Record
    if ([string]$record.release -cne $old.tag) {$failures.Add("the record names $($record.release), not $($old.tag)")}
    $hash=(Get-FileHash -LiteralPath $paths.Program -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($hash -cne [string]$old.binaries.(Get-PlatformKey)) {$failures.Add("the installed program is not the $($old.tag) build")}
    Write-Json (Join-Path $WorkDir 'old-record.json') $record
    Assert-NoFailures 'install-old' $failures
}
# `mmhmem setup --release-dir <folder>` in the terminal, choosing the update.
function Invoke-UpdateMenu([string]$Name,[string]$Folder,[string]$Tag,[string[]]$Then) {
    $paths=Get-InstalledPaths
    $steps=@('timeout 240',"expect Newest release: $(ConvertTo-TagPattern $Tag)","select >\s+Update to $(ConvertTo-TagPattern $Tag)",'send enter')+$Then
    $null=Invoke-Pty $Name $paths.Program @('setup','--release-dir',(Join-Path $script:Site $Folder)) $steps
}
function Invoke-BadSignaturePhase {
    $bad=Get-ReleaseInfo 'badsig'
    $before=Get-HomeState 'before-bad-signature'
    Invoke-UpdateMenu 'bad-signature' $bad.folder $bad.tag @(
        'timeout 300','expect The update did not finish','expect (?i)signature','send q','timeout 60','exit 2')
    $failures=@(Compare-HomeState $before (Get-HomeState 'after-bad-signature'))
    Assert-NoFailures 'bad-signature' $failures
}
function Invoke-UpdatePhase {
    $current=Get-ReleaseInfo
    $paths=Get-InstalledPaths
    $oldRecord=Read-Json (Join-Path $WorkDir 'old-record.json')
    Invoke-UpdateMenu 'update' $current.tag $current.tag @(
        'timeout 600','expect Review: this is what will change|Everything is current','send enter',
        'timeout 900','expect enter or q close','send enter','timeout 60','exit 0')
    $failures=New-Object Collections.Generic.List[string]
    $record=Read-Json $paths.Record
    $hash=(Get-FileHash -LiteralPath $paths.Program -Algorithm SHA256).Hash.ToLowerInvariant()
    $want=[string]$current.binaries.(Get-PlatformKey)
    if ($hash -cne $want) {$failures.Add("the program was not replaced by the $($current.tag) build")}
    if ([string]$record.binary.sha256 -cne $want) {$failures.Add('the record does not hold the new program hash')}
    if ([string]$record.release -cne $current.tag) {$failures.Add("the record names $($record.release), not $($current.tag)")}
    $was=Get-AgentChoices $oldRecord;$now=Get-AgentChoices $record
    if ((ConvertTo-CanonicalJson $was) -cne (ConvertTo-CanonicalJson $now)) {$failures.Add("the choices changed: $(ConvertTo-CanonicalJson $was) -> $(ConvertTo-CanonicalJson $now)")}
    foreach ($agent in $script:ExpectedAgents) {
        $entry=$record.agents.$agent
        if (-not $entry -or [string]$entry.last_check.result -cne 'pass') {$failures.Add("$agent did not pass its check after the update");continue}
        if ([string]$entry.last_check.at -ceq [string]$oldRecord.agents.$agent.last_check.at) {$failures.Add("$agent was not checked again after the update")}
    }
    $seeded=Read-HomeState 'seeded'
    $expected=@($script:ManagedSeedFiles | Where-Object {$_ -cne '.claude.json'})+@((Get-MmhmemPatterns)+'HKCU:Path')
    foreach ($f in (Compare-HomeState $seeded (Get-HomeState 'after-update') $expected -ClaudeMode installed)) {$failures.Add($f)}
    foreach ($p in (Wait-NoLeftoverProcesses)) {$failures.Add("still running after setup finished: $p")}
    Assert-NoFailures 'update' $failures
}
# Everything mmhmem may add to a home when it installs: the program's
# folder (Windows may briefly keep the replaced program there after an
# update), the install record's folder and the memory store.
function Get-MmhmemPatterns {
    $rel=Get-MmhmemRelativePaths
    $folder=$rel.Program.Substring(0,$rel.Program.LastIndexOf('/'))
    return @("$folder/*","$($rel.RecordDir)/*","$($rel.DataDir)/*")
}

# ------------------------------------------------------ invalid-config ---
$script:InvalidOpenCodeJson="{`n  ""theme"": ""dark"",`n  ""mcp"": {`n    ""servers"": {`n"
function Invoke-InvalidConfigPhase {
    Start-Server
    $file=Join-Path $script:SeedHome '.config/opencode/opencode.json'
    Write-Utf8 $file $script:InvalidOpenCodeJson
    $before=Get-HomeState 'before-invalid-config'
    $code=Invoke-OneLinerNoTerminal 'invalid-config' (Get-ReleaseInfo).base_url
    $failures=New-Object Collections.Generic.List[string]
    if ($code -eq 0) {$failures.Add('the run exited with 0 although OpenCode was skipped')}
    if ((Read-Log 'invalid-config') -notmatch 'OpenCode was skipped: [^\r\n]*opencode\.json') {$failures.Add('the output does not say that OpenCode was skipped because of opencode.json')}
    $paths=Get-InstalledPaths
    $record=if (Test-Path -LiteralPath $paths.Record) {Read-Json $paths.Record} else {$null}
    foreach ($agent in @('claude_code','codex')) {
        if (-not $record -or [string]$record.agents.$agent.last_check.result -cne 'pass') {$failures.Add("$agent was not installed and checked")}
    }
    if ($record -and $record.agents.opencode -and @((Get-AgentChoices $record).Keys) -ccontains 'opencode') {$failures.Add('OpenCode was set up although its config is invalid')}
    # OpenCode's files (both config files included) are byte-identical; only
    # Claude Code's and Codex's server lists and mmhmem's own files change.
    $expected=@('.codex/config.toml')+@(Get-MmhmemPatterns)+'HKCU:Path'
    foreach ($f in (Compare-HomeState $before (Get-HomeState 'after-invalid-config') $expected -ClaudeMode installed)) {$failures.Add($f)}
    foreach ($p in (Wait-NoLeftoverProcesses)) {$failures.Add("still running after setup finished: $p")}
    Assert-NoFailures 'invalid-config' $failures
}
