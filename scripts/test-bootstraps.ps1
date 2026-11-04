#Requires -Version 5.1
<#
Offline checks for the rendered bootstrap scripts (task 5.3). No GitHub.
Serves rendered scripts and fake-mmhmem archives from a local Python HTTP
server, then runs install.ps1 through `irm | iex` under pwsh 7 and Windows
PowerShell 5.1, and install.sh under sh/bash/dash (via test-bootstrap-sh.sh,
with a stubbed uname). Shells that are not installed are reported and skipped.
#>
[CmdletBinding()]
param([string[]]$PosixShells=@('sh','bash','dash'),[switch]$SkipWindows)
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'render-bootstraps.ps1')
$root=Join-Path ([IO.Path]::GetTempPath()) ('bootstraps-'+[guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $root | Out-Null
$script:Failures=New-Object System.Collections.Generic.List[string]
$script:Checks=0
function Check([bool]$Condition,[string]$What) {
    $script:Checks++
    if (-not $Condition) {$script:Failures.Add($What);Write-Host "FAIL: $What"}
}
function Rejects([scriptblock]$Action) {try {& $Action} catch {return $true};return $false}
$tag='v1.2.3'
$server=$null
try {
    # ---- fixtures: fake binaries and real archives -------------------------
    $bin=Join-Path $root 'bin';New-Item -ItemType Directory -Path $bin | Out-Null
    $sources=@{}
    foreach ($key in @('linux','macos-x86_64','macos-aarch64')) {
        $text="#!/bin/sh`n# fake mmhmem for $key`n{ for a in `"`$@`"; do printf '%s\n' `"`$a`"; done; } > `"`$FAKE_LOG.args`"`nprintf '%s\n' `"`$0`" > `"`$FAKE_LOG.self`"`nif [ -t 0 ]; then echo tty; else echo notty; fi > `"`$FAKE_LOG.stdin`"`nexit `"`${FAKE_EXIT:-0}`"`n"
        $p=Join-Path $bin "mmhmem-$key";[IO.File]::WriteAllText($p,$text,(New-Object Text.UTF8Encoding($false)));$sources[$key]=$p
    }
    # A real Windows executable: compiled by Windows PowerShell 5.1.
    $fakeExe=Join-Path $bin 'mmhmem.exe'
    $csharp=Join-Path $bin 'fake.cs'
    [IO.File]::WriteAllText($csharp,@'
using System;using System.IO;using System.Reflection;
public static class P{public static int Main(string[] a){
 string log=Environment.GetEnvironmentVariable("FAKE_LOG");
 File.WriteAllText(log+".args",string.Join("\n",a)+"\n");
 File.WriteAllText(log+".self",Assembly.GetExecutingAssembly().Location+"\n");
 string e=Environment.GetEnvironmentVariable("FAKE_EXIT");
 return string.IsNullOrEmpty(e)?0:int.Parse(e);}}
'@)
    $compile="Add-Type -Path '$csharp' -OutputAssembly '$fakeExe' -OutputType ConsoleApplication"
    $ErrorActionPreference='Continue'
    & powershell.exe -NoProfile -NonInteractive -Command $compile
    $ErrorActionPreference='Stop'
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path $fakeExe)) {throw 'Could not compile the fake mmhmem.exe'}
    $sources['windows']=$fakeExe

    $site=Join-Path $root 'site'
    $dl=Join-Path $site "dl/$tag"
    $null=New-ReleaseArchives $tag $sources $dl
    New-Item -ItemType Directory -Path (Join-Path $site 'ok'),(Join-Path $site 'badhash'),(Join-Path $site 'missing'),(Join-Path $site 'dl/nope') | Out-Null

    # ---- local server -------------------------------------------------------
    $listener=New-Object Net.Sockets.TcpListener([Net.IPAddress]::Loopback,0);$listener.Start()
    $port=$listener.LocalEndpoint.Port;$listener.Stop()
    $python=(& py -3.12 -I -c 'import sys;print(sys.executable)').Trim()
    $logPath=Join-Path $site 'server.log'
    $server=Start-Process -FilePath $python -PassThru -WindowStyle Hidden -RedirectStandardError $logPath -ArgumentList @('-I','-m','http.server',"$port",'--bind','127.0.0.1','--directory',$site)
    $up=$false
    for ($i=0;$i -lt 50 -and -not $up;$i++) {
        try {$c=New-Object Net.Sockets.TcpClient('127.0.0.1',$port);$c.Close();$up=$true} catch {Start-Sleep -Milliseconds 100}
    }
    if (-not $up) {throw 'Local HTTP server did not start'}
    $base="http://127.0.0.1:$port/dl/$tag"

    # ---- renderer -----------------------------------------------------------
    $null=Write-RenderedBootstraps $tag $dl (Join-Path $site 'ok') $base
    foreach ($name in @('install.sh','install.ps1')) {
        $bytes=[IO.File]::ReadAllBytes((Join-Path $site "ok/$name"))
        $text=[Text.Encoding]::UTF8.GetString($bytes)
        Check ($text -notmatch '@@') "$name has no leftover placeholder"
        Check ($text -notmatch "`r") "$name is LF-only"
        Check ($bytes.Length -gt 2 -and $bytes[0] -ne 0xEF) "$name has no BOM"
        Check ($text.Contains("'$tag'")) "$name embeds the tag"
        Check ($text.Contains($base)) "$name embeds the base URL"
        Check ($text -notmatch '(?i)claude|codex|opencode|antigravity|cursor') "$name names no agents"
        Check ($text -notmatch '(?i)\bparam\s*\(|getopts|\$args|\$@|--agents') "$name parses no options"
    }
    $sh=[IO.File]::ReadAllText((Join-Path $site 'ok/install.sh'))
    $ps=[IO.File]::ReadAllText((Join-Path $site 'ok/install.ps1'))
    foreach ($t in (Get-ReleaseArchiveTargets $tag)) {
        $h=(Get-FileHash -LiteralPath (Join-Path $dl $t.Archive) -Algorithm SHA256).Hash.ToLowerInvariant()
        $in=if ($t.Key -ceq 'windows') {$ps} else {$sh}
        Check $in.Contains($h) "script embeds the hash of $($t.Archive)"
    }
    Check ($sh -match 'setup --bootstrap "\$TAG"') 'install.sh calls setup --bootstrap <tag>'
    Check ($ps -match 'setup --bootstrap \$tag') 'install.ps1 calls setup --bootstrap <tag>'
    Check ($ps -notmatch '(?i)\btar(\.exe)?\b') 'install.ps1 never mentions tar'
    Check ($ps -match 'Expand-Archive' -and $ps -match 'UseBasicParsing' -and $ps -match 'Tls12') 'install.ps1 uses Expand-Archive, basic parsing and TLS 1.2'
    Check ($sh -match 'sha256sum' -and $sh -match 'shasum -a 256' -and $sh -match 'wget' -and $sh -match '/dev/tty') 'install.sh has sha256sum/shasum, wget and /dev/tty fallbacks'
    # Default base URL is the GitHub release URL of the tag.
    $null=Write-RenderedBootstraps $tag $dl (Join-Path $root 'default')
    Check ([IO.File]::ReadAllText((Join-Path $root 'default/install.ps1')).Contains("'https://github.com/norandom/mmhmem/releases/download/$tag'")) 'default base URL is the GitHub release download URL'
    # Refusals.
    Check (Rejects {Write-RenderedBootstraps $tag (Join-Path $root 'nowhere') (Join-Path $root 'x1')}) 'missing archives are refused'
    Check (-not (Test-Path (Join-Path $root 'x1'))) 'refusal leaves no partial output'
    $partial=Join-Path $root 'partial';Copy-Item $dl $partial -Recurse
    Remove-Item (Join-Path $partial "mmhmem-$tag-aarch64-apple-darwin.tar.gz")
    Check (Rejects {Write-RenderedBootstraps $tag $partial (Join-Path $root 'x2')}) 'one missing hash is refused'
    foreach ($badUrl in @("http://x/'; rm -rf ~; '",'ftp://x/y','http://a b/c','https://x/y/',"http://x/`$(id)")) {
        Check (Rejects {Write-RenderedBootstraps $tag $dl (Join-Path $root 'x3') $badUrl}) "unsafe base URL refused: $badUrl"
    }
    Check (Rejects {Write-RenderedBootstraps 'v1.2' $dl (Join-Path $root 'x4')}) 'bad tag refused'
    $tpl=Join-Path $root 'tpl';New-Item -ItemType Directory -Path $tpl | Out-Null
    $realTpl=Get-BootstrapTemplateDirectory
    function Get-BootstrapTemplateDirectory {$tpl}
    foreach ($n in @('install.sh.tmpl','install.ps1.tmpl')) {Copy-Item (Join-Path $realTpl $n) (Join-Path $tpl $n)}
    Add-Content -LiteralPath (Join-Path $tpl 'install.sh.tmpl') -Value '# @@MYSTERY@@'
    Check (Rejects {Write-RenderedBootstraps $tag $dl (Join-Path $root 'x5') $base}) 'unknown placeholder is refused'
    Check (-not (Test-Path (Join-Path $root 'x5'))) 'unknown placeholder leaves no output (not even install.ps1)'
    Copy-Item (Join-Path $realTpl 'install.sh.tmpl') (Join-Path $tpl 'install.sh.tmpl') -Force
    Add-Content -LiteralPath (Join-Path $tpl 'install.sh.tmpl') -Value '# a stray @@ marker'
    Check (Rejects {Write-RenderedBootstraps $tag $dl (Join-Path $root 'x6') $base}) 'stray @@ is refused'
    function Get-BootstrapTemplateDirectory {Join-Path (Split-Path -Parent $PSScriptRoot) 'deploy/bootstrap'}

    # ---- variants: wrong hash, 404 -----------------------------------------
    $zeros='0'*64
    $winHash=(Get-FileHash -LiteralPath (Join-Path $dl "mmhmem-$tag-x86_64-pc-windows-msvc.zip") -Algorithm SHA256).Hash.ToLowerInvariant()
    Copy-Item (Join-Path $site 'ok/*') (Join-Path $site 'badhash')
    foreach ($t in (Get-ReleaseArchiveTargets $tag)) {
        $h=(Get-FileHash -LiteralPath (Join-Path $dl $t.Archive) -Algorithm SHA256).Hash.ToLowerInvariant()
        foreach ($n in @('install.sh','install.ps1')) {
            $f=Join-Path $site "badhash/$n"
            [IO.File]::WriteAllText($f,[IO.File]::ReadAllText($f).Replace($h,$zeros),(New-Object Text.UTF8Encoding($false)))
        }
    }
    $null=Write-RenderedBootstraps $tag $dl (Join-Path $site 'missing') "http://127.0.0.1:$port/dl/nope"

    # ---- Windows: irm | iex under pwsh 7 and powershell 5.1 -----------------
    function Read-ServerLines {
        $fs=[IO.File]::Open($logPath,'Open','Read','ReadWrite');$r=New-Object IO.StreamReader($fs)
        try {return @($r.ReadToEnd() -split "`n" | Where-Object {$_.Trim()})} finally {$r.Dispose()}
    }
    function Invoke-Installer([string]$Shell,[string]$Variant,[int]$FakeExit,[string]$Pre,[string]$PathPrefix) {
        $ErrorActionPreference='Continue'   # Windows PowerShell 5.1 turns native stderr into errors under Stop
        $work=Join-Path $root ('run-'+[guid]::NewGuid().ToString('N'));$tmp=Join-Path $work 'tmp'
        New-Item -ItemType Directory -Path $tmp -Force | Out-Null
        $set=@{TEMP=$tmp;TMP=$tmp;FAKE_LOG=(Join-Path $work 'fake');FAKE_EXIT="$FakeExit"}
        if ($PathPrefix) {$set['PATH']="$PathPrefix;$env:PATH"}
        $old=@{};foreach ($k in $set.Keys) {$old[$k]=[Environment]::GetEnvironmentVariable($k);[Environment]::SetEnvironmentVariable($k,$set[$k])}
        $before=(Read-ServerLines).Count
        try {
            if (-not $Pre) {$Pre='$null'}
            $cmd="$Pre; try { irm 'http://127.0.0.1:$port/$Variant/install.ps1' | iex } catch { Write-Host (`"CAUGHT: `"+`$_.Exception.Message) }; exit `$LASTEXITCODE"
            $out=@(& $Shell -NoProfile -NonInteractive -Command $cmd 2>&1 | ForEach-Object {"$_"})
            $code=$LASTEXITCODE
        } finally {foreach ($k in $old.Keys) {[Environment]::SetEnvironmentVariable($k,$old[$k])}}
        return [pscustomobject]@{Code=$code;Out=($out -join "`n");Dir=$work;Tmp=$tmp;Requests=@((Read-ServerLines)[$before..((Read-ServerLines).Count-1)] | Where-Object {$_})}
    }
    $shells=@()
    if (Get-Command pwsh -ErrorAction SilentlyContinue) {$shells+=,@('pwsh 7','pwsh')} else {Write-Host 'SKIP: pwsh not installed'}
    $shells+=,@('powershell 5.1','powershell.exe')
    $shim=Join-Path $root 'shim';New-Item -ItemType Directory -Path $shim | Out-Null
    [IO.File]::WriteAllText((Join-Path $shim 'tar.cmd'),"@echo off`r`necho called> `"%FAKE_LOG%.tarcalled`"`r`n")
    foreach ($entry in $(if ($SkipWindows) {@()} else {$shells})) {
        $label=$entry[0];$exe=$entry[1]
        foreach ($code in @(0,7)) {
            $r=Invoke-Installer $exe 'ok' $code '' $shim
            Check ($r.Code -eq $code) "$label happy path passes exit code $code through (got $($r.Code)): $($r.Out)"
            Check ((Test-Path "$($r.Dir)/fake.args") -and ([IO.File]::ReadAllText("$($r.Dir)/fake.args") -ceq "setup`n--bootstrap`n$tag`n")) "$label calls exactly 'setup --bootstrap $tag'"
            Check ((Test-Path "$($r.Dir)/fake.self") -and ([IO.File]::ReadAllText("$($r.Dir)/fake.self").StartsWith($r.Tmp,[StringComparison]::OrdinalIgnoreCase))) "$label ran setup from the temp folder"
            Check (-not (Test-Path "$($r.Dir)/fake.tarcalled")) "$label never calls tar"
            Check (@(Get-ChildItem $r.Tmp -Force).Count -eq 0) "$label removed its temp folder (exit $code)"
            Check ($r.Out -match 'SHA-256 matches' -and $r.Out -match 'Installing mmhmem') "$label tells the user what it does and that it checked"
            Check (($r.Requests -join "`n") -match "GET /dl/$tag/mmhmem-$tag-x86_64-pc-windows-msvc\.zip") "$label requested the Windows zip"
        }
        $r=Invoke-Installer $exe 'badhash' 0 '' $shim
        Check ($r.Code -ne 0) "$label hash mismatch exits non-zero"
        Check (-not (Test-Path "$($r.Dir)/fake.args")) "$label hash mismatch never runs setup"
        Check ($r.Out.Contains("expected SHA-256: $zeros") -and $r.Out -match "actual SHA-256:\s+$winHash") "$label hash mismatch shows expected and actual"
        Check ($r.Out -match 'Nothing was installed or changed') "$label hash mismatch says nothing changed"
        Check (@(Get-ChildItem $r.Tmp -Force).Count -eq 0) "$label hash mismatch removed the temp folder"
        $r=Invoke-Installer $exe 'missing' 0 '' $shim
        Check ($r.Code -ne 0 -and $r.Out -match 'download failed' -and $r.Out -match 'Nothing was installed') "$label 404 gives a plain message and non-zero exit"
        Check (-not (Test-Path "$($r.Dir)/fake.args")) "$label 404 never runs setup"
        Check (@(Get-ChildItem $r.Tmp -Force).Count -eq 0) "$label 404 removed the temp folder"
        $r=Invoke-Installer $exe 'ok' 0 '$env:PROCESSOR_ARCHITECTURE=''ARM64''; $env:PROCESSOR_ARCHITEW6432=$null' $shim
        $archReq=@($r.Requests | Where-Object {$_ -match 'GET /dl/'})
        Check ($r.Code -ne 0 -and $r.Out -match 'does not have a download' -and $archReq.Count -eq 0) "$label unsupported CPU says so and downloads nothing"
        Check (-not (Test-Path "$($r.Dir)/fake.args") -and @(Get-ChildItem $r.Tmp -Force).Count -eq 0) "$label unsupported CPU changes nothing"
    }

    # ---- POSIX: sh, bash, dash under Git Bash -------------------------------
    $gitBash=$null
    $gitCmd=Get-Command git -ErrorAction SilentlyContinue
    if ($gitCmd) {$cand=Join-Path (Split-Path -Parent (Split-Path -Parent $gitCmd.Source)) 'bin/bash.exe';if (Test-Path $cand) {$gitBash=$cand}}
    $bash=if ($gitBash) {[pscustomobject]@{Source=$gitBash}} else {$null}
    if ($bash) {
        function Get-BashLine([string]$Command) {   # retries Git Bash fork flakes
            $ErrorActionPreference='Continue'
            for ($i=0;$i -lt 10;$i++) {
                $o=@(& $bash.Source -c $Command 2>$null | ForEach-Object {"$_"})
                if ($LASTEXITCODE -eq 0 -and $o.Count) {return $o[0].Trim()}
                Start-Sleep -Milliseconds 300
            }
            return ''
        }
        $toPosix={param($p) Get-BashLine "cygpath -u '$p'"}
        $fixPosix=& $toPosix $site
        $runner=& $toPosix (Join-Path $PSScriptRoot 'test-bootstrap-sh.sh')
        foreach ($shellName in $PosixShells) {
            $present=Get-BashLine "command -v $shellName >/dev/null 2>&1 && echo yes"
            if ($present -ne 'yes') {Write-Host "SKIP: $shellName not available";continue}
            # The runner itself is POSIX sh; the shell under test runs install.sh.
            # Git Bash forks sometimes fail with 'Resource temporarily unavailable'
            # (cygheap child_copy); that is the host, not the script, so retry.
            $ErrorActionPreference='Continue'
            for ($attempt=1;$attempt -le 10;$attempt++) {
                $result=@(& $bash.Source -c "sh '$runner' $shellName '$fixPosix' $port" 2>&1 | ForEach-Object {"$_"})
                $code=$LASTEXITCODE
                if ($code -eq 0 -or -not ($result -match 'child_copy|fork: retry|dofork')) {break}
                Write-Host "  (Git Bash fork failure, retrying $attempt)"
            }
            $result | Where-Object {$_ -notmatch 'child_copy|dofork'} | ForEach-Object {Write-Host "  $_"}
            Check ($code -eq 0) "POSIX checks pass under $shellName"
        }
    } else {Write-Host 'SKIP: no Git Bash; POSIX checks not run'}
}
finally {
    if ($server -and -not $server.HasExited) {Stop-Process -Id $server.Id -Force}
    Start-Sleep -Milliseconds 200
    Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
}
if ($script:Failures.Count) {Write-Host "$($script:Failures.Count) of $($script:Checks) checks FAILED";exit 1}
Write-Host "PASS: bootstraps render, verify, stop cleanly and pass setup's exit code ($($script:Checks) checks)"
