#Requires -Version 5.1
<#
.SYNOPSIS
Driver for the installer end-to-end job (.github/workflows/installer-e2e.yml,
reliable-installers tasks 8.1 and 8.2, Requirement 10).
.DESCRIPTION
One phase per call, so each is a separate, readable CI step. State lives in
-WorkDir (default $env:MMHMEM_E2E_WORKDIR, else <RUNNER_TEMP or TEMP>/mmhmem-e2e):

  site/<tag>/       archives, install.sh, install.ps1, SHA256SUMS(.sig): served
  site/release.json tag, port, base URL and public key of this e2e release
  home/             the clean user home (seeded)
  seed-manifest.sha256, hkcu-path.json (Windows), state.json, logs/

Phases (8.1 implements the first group; the second is task 8.2):

  keygen            new ephemeral zipsign key pair (-KeyFile); prints and
                    exports (GITHUB_OUTPUT public_key) the public key hex
  release           package, sign, render (-BaseUrl http://127.0.0.1:<port>/<tag>),
                    hash and sign SHA256SUMS with the SAME scripts the release
                    uses, from -BinaryDirectory (mmhmem.exe, mmhmem-linux-x86_64,
                    mmhmem-macos-x86_64, mmhmem-macos-aarch64) and -KeyFile
  seed              create and seed home/, write the SHA-256 manifest and, on
                    Windows, snapshot HKCU\Environment\Path
  serve             start a local HTTP server (python -m http.server) on site/
  install           run the exact user one-liner without a terminal:
                      curl -fsSL <url>/install.sh | sh
                      irm <url>/install.ps1 | iex
                    in the isolated environment of home/ (default path,
                    amended Requirement 4.9: the bootstrap passes --bootstrap)
  assert-installed  exit 0, install record and program in place, and every
                    seeded file not managed by mmhmem byte-identical
  compare-seed      compare home/ (and HKCU Path) with the manifest;
                    -AllowChanged lists seeded paths that may differ
  stop              stop the server; restore HKCU Path if it changed

  menu-install rerun update remove cancel-at-review download-404 bad-hash
  bad-signature invalid-config      -> task 8.2 (stubs; they fail loudly)

Isolation (see the workflow header): Linux/macOS get a fresh HOME and TMPDIR,
with XDG_*, CLAUDE_CONFIG_DIR and CODEX_HOME removed so every tool derives its
paths from HOME as on a real machine. Windows cannot get a fresh profile
without admin rights, so USERPROFILE, HOME, HOMEDRIVE/HOMEPATH, APPDATA,
LOCALAPPDATA and TEMP/TMP point into home/, and CODEX_HOME is set to
home\.codex because codex.exe may resolve the profile through the Known
Folder API instead of USERPROFILE. HKCU stays the runner's: the PATH value is
snapshotted by `seed`, compared by `compare-seed` and restored by `stop`.

Windows PowerShell 5.1 and PowerShell 7 (Windows, Linux, macOS) compatible.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)]
    [ValidateSet('keygen','release','seed','serve','install','assert-installed','compare-seed','stop',
        'menu-install','rerun','update','remove','cancel-at-review','download-404','bad-hash','bad-signature','invalid-config')]
    [string]$Phase,
    [string]$WorkDir,
    [string]$Tag,
    [int]$Port=47801,
    [string]$BinaryDirectory,
    [string]$KeyFile,
    [string[]]$AllowChanged=@(),
    [int]$TimeoutSeconds=900
)
$ErrorActionPreference='Stop'
$script:Root=Split-Path -Parent $PSScriptRoot
$script:OnWindows=[Environment]::OSVersion.Platform -eq [PlatformID]::Win32NT
$script:OnMac=(-not $script:OnWindows) -and ((& uname -s) -ceq 'Darwin')
. (Join-Path $PSScriptRoot 'render-bootstraps.ps1')   # also loads package-release-archives.ps1
. (Join-Path $PSScriptRoot 'sign-release-assets.ps1')
. (Join-Path $PSScriptRoot 'e2e-seed-home.ps1')

if (-not $WorkDir) {
    $WorkDir=$env:MMHMEM_E2E_WORKDIR
    if (-not $WorkDir) {
        $base=$env:RUNNER_TEMP
        if (-not $base) {$base=[IO.Path]::GetTempPath()}
        $WorkDir=Join-Path $base 'mmhmem-e2e'
    }
}
New-Item -ItemType Directory -Path $WorkDir -Force | Out-Null
$WorkDir=(Resolve-Path -LiteralPath $WorkDir).ProviderPath
$script:Site=Join-Path $WorkDir 'site'
$script:SeedHome=Join-Path $WorkDir 'home'
$script:Logs=Join-Path $WorkDir 'logs'
$script:Manifest=Join-Path $WorkDir 'seed-manifest.sha256'
$script:RegistrySnapshot=Join-Path $WorkDir 'hkcu-path.json'
$script:StateFile=Join-Path $WorkDir 'state.json'
New-Item -ItemType Directory -Path $script:Logs -Force | Out-Null

function Write-Utf8([string]$Path,[string]$Text) {[IO.File]::WriteAllText($Path,$Text,(New-Object Text.UTF8Encoding($false)))}
function Read-Json([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {throw "Missing $Path (run the earlier phase first)"}
    return ([IO.File]::ReadAllText($Path) | ConvertFrom-Json)
}
function Write-Json([string]$Path,$Value) {Write-Utf8 $Path (($Value | ConvertTo-Json -Depth 8)+"`n")}
function Add-GitHubOutput([string]$Name,[string]$Value) {
    if ($env:GITHUB_OUTPUT) {[IO.File]::AppendAllText($env:GITHUB_OUTPUT,"$Name=$Value`n",(New-Object Text.UTF8Encoding($false)))}
}
function Get-Release {Read-Json (Join-Path $script:Site 'release.json')}

# ---------------------------------------------------------------- keygen ---
function Invoke-KeygenPhase {
    if (-not $KeyFile) {throw '-KeyFile is required'}
    Assert-ZipsignVersion
    if (Test-Path -LiteralPath $KeyFile) {throw "$KeyFile already exists; an ephemeral key is never reused"}
    $parent=Split-Path -Parent $KeyFile
    if ($parent -and -not (Test-Path -LiteralPath $parent)) {New-Item -ItemType Directory -Path $parent -Force | Out-Null}
    $public=$KeyFile+'.pub'
    $null=Invoke-Zipsign @('gen-key',$KeyFile,$public)
    $hex=(([IO.File]::ReadAllBytes($public) | ForEach-Object {$_.ToString('x2')}) -join '')
    if ($hex.Length -ne 64) {throw 'Unexpected public key length'}
    Write-Utf8 ($KeyFile+'.pub.hex') $hex
    Add-GitHubOutput 'public_key' $hex
    Write-Host "Ephemeral e2e release key: $hex"
}

# --------------------------------------------------------------- release ---
# A release_keys.rs-shaped file listing only the ephemeral key, so
# Open-ReleaseSigning's fail-closed "key must be embedded" check passes for
# the e2e build (which embeds this key, see the workflow) and nothing else.
function Write-E2EKeyList([string]$PublicHex,[string]$Path) {
    $bytes=for ($i=0;$i -lt 64;$i+=2) {'0x'+$PublicHex.Substring($i,2)}
    Write-Utf8 $Path ("pub const RELEASE_PUBLIC_KEYS: &[[u8; 32]] = &[`n    [$($bytes -join ', ')],`n];`n")
}
function Invoke-ReleasePhase {
    if (-not $Tag -or -not $BinaryDirectory -or -not $KeyFile) {throw '-Tag, -BinaryDirectory and -KeyFile are required'}
    $names=[ordered]@{'windows'='mmhmem.exe';'linux'='mmhmem-linux-x86_64';'macos-x86_64'='mmhmem-macos-x86_64';'macos-aarch64'='mmhmem-macos-aarch64'}
    $sources=@{}
    foreach ($key in $names.Keys) {$sources[$key]=Join-Path $BinaryDirectory $names[$key]}
    $baseUrl="http://127.0.0.1:$Port/$Tag"
    $tagDir=Join-Path $script:Site $Tag
    if (Test-Path -LiteralPath $script:Site) {Remove-Item -LiteralPath $script:Site -Recurse -Force}
    $scratch=Join-Path $WorkDir ('signing-'+[guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $scratch | Out-Null
    $signing=$null
    try {
        $public=Join-Path $scratch 'derived.pub'
        $null=Invoke-Zipsign @('gen-key','--extract',$KeyFile,$public,'--force')
        $publicHex=(([IO.File]::ReadAllBytes($public) | ForEach-Object {$_.ToString('x2')}) -join '')
        $keyList=Join-Path $scratch 'release_keys.rs'
        Write-E2EKeyList $publicHex $keyList
        $signing=Open-ReleaseSigning (Join-Path $scratch 'key') ([Convert]::ToBase64String([IO.File]::ReadAllBytes($KeyFile))) $keyList
        # Same order as publish-ci-release.ps1: archives, sign, render, hash, sign sums.
        $archives=New-ReleaseArchives $Tag $sources $tagDir
        Invoke-ReleaseSigning $archives $signing
        $scripts=Write-RenderedBootstraps $Tag $tagDir $tagDir $baseUrl
        $null=Write-ReleaseChecksums $tagDir
        $null=Write-ReleaseChecksumSignature $tagDir $signing
        $files=[ordered]@{}
        foreach ($f in (Get-ChildItem -LiteralPath $tagDir -File | Sort-Object Name)) {
            $files[$f.Name]=(Get-FileHash -LiteralPath $f.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        }
        Write-Json (Join-Path $script:Site 'release.json') ([ordered]@{tag=$Tag;port=$Port;base_url=$baseUrl;public_key=$publicHex;files=$files})
        Write-Host "Release $Tag staged in $tagDir ($(@($archives).Count) signed archives, $($scripts -join ', '), SHA256SUMS, SHA256SUMS.sig)"
        Write-Host "Base URL: $baseUrl"
    } finally {
        Close-ReleaseSigning $signing
        Remove-Item -LiteralPath $scratch -Recurse -Force -ErrorAction SilentlyContinue
    }
}

# ------------------------------------------------------------------ seed ---
function Get-UserPathSnapshot {
    $key=[Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment')
    try {
        if (-not $key -or ($key.GetValueNames() -notcontains 'Path')) {return [ordered]@{exists=$false;kind='';value=''}}
        return [ordered]@{exists=$true;kind=[string]$key.GetValueKind('Path');value=[string]$key.GetValue('Path','',[Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)}
    } finally {if ($key) {$key.Close()}}
}
function Invoke-SeedPhase {
    if (Test-Path -LiteralPath $script:SeedHome) {Remove-Item -LiteralPath $script:SeedHome -Recurse -Force}
    & (Join-Path $PSScriptRoot 'e2e-seed-home.ps1') -Destination $script:SeedHome -Manifest $script:Manifest
    $state=[ordered]@{home=$script:SeedHome;os=$(if ($script:OnWindows) {'windows'} elseif ($script:OnMac) {'macos'} else {'linux'})
        orig_home=$env:HOME;orig_userprofile=$env:USERPROFILE;orig_appdata=$env:APPDATA;orig_localappdata=$env:LOCALAPPDATA}
    if ($script:OnWindows) {
        Write-Json $script:RegistrySnapshot (Get-UserPathSnapshot)
        Write-Host 'Snapshotted HKCU\Environment\Path (the runner''s; Windows cannot get a fresh profile without admin rights)'
    }
    Write-Json $script:StateFile $state
}

# ----------------------------------------------------------------- serve ---
function Get-Python {
    foreach ($candidate in @(@('py','-3'),@('python3'),@('python'))) {
        if (Get-Command $candidate[0] -ErrorAction SilentlyContinue) {
            $path=& $candidate[0] @($candidate | Select-Object -Skip 1) -I -c 'import sys;print(sys.executable)' 2>$null
            if ($LASTEXITCODE -eq 0 -and $path) {return ([string]$path).Trim()}
        }
    }
    throw 'No Python 3 found for the local HTTP server'
}
function Test-Port([int]$Number) {
    try {$c=New-Object Net.Sockets.TcpClient('127.0.0.1',$Number);$c.Close();return $true} catch {return $false}
}
function Test-Url([string]$Url) {
    try {$r=Invoke-WebRequest -UseBasicParsing -Uri $Url -Method Head -TimeoutSec 10;return ($r.StatusCode -eq 200)} catch {return $false}
}
function Start-Server {
    $release=Get-Release
    $url="$($release.base_url)/install.sh"
    if (Test-Url $url) {Write-Host "Server already serving $($release.base_url)";return}
    if (Test-Port $release.port) {throw "Port $($release.port) is taken by another program; the bootstraps were rendered for it"}
    $python=Get-Python
    $arguments=@('-I','-m','http.server',[string]$release.port,'--bind','127.0.0.1','--directory',"`"$script:Site`"")
    $options=@{FilePath=$python;ArgumentList=$arguments;PassThru=$true
        RedirectStandardOutput=(Join-Path $script:Logs 'server.out');RedirectStandardError=(Join-Path $script:Logs 'server.log')}
    if ($script:OnWindows) {$options['WindowStyle']='Hidden'}
    $server=Start-Process @options
    Write-Utf8 (Join-Path $WorkDir 'server.pid') ([string]$server.Id)
    for ($i=0;$i -lt 100;$i++) {
        if (Test-Url $url) {Write-Host "Serving $script:Site on $($release.base_url) (pid $($server.Id))";return}
        Start-Sleep -Milliseconds 200
    }
    throw "The local HTTP server did not come up; see $(Join-Path $script:Logs 'server.log')"
}

# ------------------------------------------------- isolated environment ---
function Get-IsolatedEnvironment {
    $state=Read-Json $script:StateFile
    $h=$state.home
    $vars=[ordered]@{}
    foreach ($name in @('XDG_CONFIG_HOME','XDG_DATA_HOME','XDG_STATE_HOME','XDG_CACHE_HOME','CLAUDE_CONFIG_DIR','CODEX_HOME','OPENCODE_CONFIG','OPENCODE_CONFIG_DIR','ZDOTDIR')) {$vars[$name]=$null}
    # No auto-updates or optional traffic from the agent CLIs during the run.
    $vars['DISABLE_AUTOUPDATER']='1'
    $vars['CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC']='1'
    $vars['OPENCODE_DISABLE_AUTOUPDATE']='true'
    $sep=[IO.Path]::PathSeparator
    if ($script:OnWindows) {
        $local=Join-Path $h 'AppData\Local'
        $temp=Join-Path $local 'Temp'
        New-Item -ItemType Directory -Path (Join-Path $h 'AppData\Roaming'),$temp -Force | Out-Null
        $vars['USERPROFILE']=$h;$vars['HOME']=$h
        $vars['HOMEDRIVE']=$h.Substring(0,2);$vars['HOMEPATH']=$h.Substring(2)
        $vars['APPDATA']=Join-Path $h 'AppData\Roaming';$vars['LOCALAPPDATA']=$local
        $vars['TEMP']=$temp;$vars['TMP']=$temp
        $vars['CODEX_HOME']=Join-Path $h '.codex'
        $dropExact=@()
        if ($state.orig_appdata) {$dropExact+=(Join-Path $state.orig_appdata 'npm')}
        if ($state.orig_localappdata) {$dropExact+=(Join-Path $state.orig_localappdata 'Programs\mmhmem')}
        if ($state.orig_userprofile) {$dropExact+=(Join-Path $state.orig_userprofile '.local\bin')}
    } else {
        $temp=Join-Path $WorkDir 'tmp'
        New-Item -ItemType Directory -Path $temp -Force | Out-Null
        $vars['HOME']=$h;$vars['TMPDIR']=$temp
        $dropExact=@()
        if ($state.orig_home) {$dropExact+=(Join-Path $state.orig_home '.local/bin')}
    }
    # Only the user-level program folders of the runner's own profile are
    # removed from PATH (tool caches such as setup-node live elsewhere, or
    # under the runner's home on macOS, and must stay).
    $kept=@($env:PATH -split [regex]::Escape($sep) | Where-Object {
        $entry=$_.TrimEnd('\','/')
        $entry -and -not (@($dropExact | Where-Object {$_.TrimEnd('\','/') -eq $entry}).Count)
    })
    $vars['PATH']=$kept -join $sep
    return $vars
}
# Runs a program with the isolated environment, stdin from an empty file
# (so there is no terminal), output to logs/<Name>.out|.err. Returns the exit code.
function Invoke-Isolated([string]$Name,[string]$FilePath,[string]$Arguments) {
    $vars=Get-IsolatedEnvironment
    $state=Read-Json $script:StateFile
    $saved=@{}
    foreach ($k in $vars.Keys) {$saved[$k]=[Environment]::GetEnvironmentVariable($k)}
    $stdin=Join-Path $script:Logs "$Name.stdin";Write-Utf8 $stdin ''
    $out=Join-Path $script:Logs "$Name.out";$err=Join-Path $script:Logs "$Name.err"
    try {
        foreach ($k in $vars.Keys) {[Environment]::SetEnvironmentVariable($k,$vars[$k])}
        $options=@{FilePath=$FilePath;ArgumentList=$Arguments;PassThru=$true;WorkingDirectory=$state.home
            RedirectStandardInput=$stdin;RedirectStandardOutput=$out;RedirectStandardError=$err}
        if ($script:OnWindows) {$options['NoNewWindow']=$true}
        $process=Start-Process @options
        $null=$process.Handle   # keeps ExitCode readable after exit (Windows PowerShell)
        if (-not $process.WaitForExit($TimeoutSeconds*1000)) {
            try {$process.Kill()} catch {}
            throw "$Name did not finish within $TimeoutSeconds s"
        }
        $code=$process.ExitCode
    } finally {
        foreach ($k in $saved.Keys) {[Environment]::SetEnvironmentVariable($k,$saved[$k])}
    }
    Write-Utf8 (Join-Path $script:Logs "$Name.exit") ([string]$code)
    Write-Host "----- $Name output (exit $code) -----"
    foreach ($file in @($out,$err)) {if ((Test-Path -LiteralPath $file) -and (Get-Item -LiteralPath $file).Length) {Get-Content -LiteralPath $file -Encoding UTF8 | ForEach-Object {Write-Host $_}}}
    return $code
}

# --------------------------------------------------------------- install ---
# The exact user one-liner. -NoProfile/-NonInteractive keep the runner's
# profile out; `exit $LASTEXITCODE` only hands setup's exit code to the
# harness (install.ps1 never calls exit itself, so a terminal stays open).
function Get-OneLiner([string]$BaseUrl) {
    if ($script:OnWindows) {
        $shell=(Get-Process -Id $PID).Path   # same edition as this step: powershell.exe 5.1 or pwsh
        return @{File=$shell;Arguments="-NoProfile -NonInteractive -Command `"irm '$BaseUrl/install.ps1' | iex; exit `$LASTEXITCODE`""}
    }
    $line="curl -fsSL '$BaseUrl/install.sh' | sh"
    # setsid: no controlling terminal, so the bootstrap's /dev/tty probe fails
    # as it does for a user piping into sh from a non-interactive session.
    if (Get-Command setsid -ErrorAction SilentlyContinue) {return @{File='setsid';Arguments="-w /bin/sh -c `"$line`""}}
    return @{File='/bin/sh';Arguments="-c `"$line`""}
}
function Invoke-InstallPhase {
    Start-Server
    $release=Get-Release
    $command=Get-OneLiner $release.base_url
    Write-Host "Running the one-liner: $($command.Arguments)"
    $code=Invoke-Isolated 'install' $command.File $command.Arguments
    if ($code -ne 0) {throw "The install one-liner exited with $code"}
}

# ------------------------------------------------------ assert-installed ---
function Get-ExpectedLayout {
    $h=$script:SeedHome
    if ($script:OnWindows) {return @{Record=(Join-Path $h 'AppData\Local\mmhmem\install.json');Binary=(Join-Path $h 'AppData\Local\Programs\mmhmem\mmhmem.exe')}}
    if ($script:OnMac) {return @{Record=(Join-Path $h 'Library/Application Support/mmhmem/install.json');Binary=(Join-Path $h '.local/bin/mmhmem')}}
    return @{Record=(Join-Path $h '.local/state/mmhmem/install.json');Binary=(Join-Path $h '.local/bin/mmhmem')}
}
# Seeded files the default install may change: the three agents' MCP entries.
# OpenCode: setup edits the file that already holds an mmhmem entry, else
# opencode.jsonc (opencode.rs); the seed has both files, neither with mmhmem.
$script:ManagedSeedFiles=@('.claude.json','.codex/config.toml','.config/opencode/opencode.jsonc')
# Install record keys of the agents the job installs (claude, codex, opencode).
$script:ExpectedAgents=@('claude_code','codex','opencode')
function Invoke-AssertInstalledPhase {
    $exitFile=Join-Path $script:Logs 'install.exit'
    if (-not (Test-Path -LiteralPath $exitFile)) {throw 'No install run recorded'}
    $failures=New-Object System.Collections.Generic.List[string]
    if (([IO.File]::ReadAllText($exitFile)).Trim() -ne '0') {$failures.Add('install exit code is not 0')}
    $layout=Get-ExpectedLayout
    if (-not (Test-Path -LiteralPath $layout.Record -PathType Leaf)) {$failures.Add("no install record at $($layout.Record)")}
    if (-not (Test-Path -LiteralPath $layout.Binary -PathType Leaf)) {$failures.Add("no program at $($layout.Binary)")}
    if (-not $failures.Count) {
        $record=Read-Json $layout.Record
        $hash=(Get-FileHash -LiteralPath $layout.Binary -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($record.binary -and $record.binary.sha256 -and ([string]$record.binary.sha256).ToLowerInvariant() -cne $hash) {$failures.Add('the program does not match the SHA-256 in the install record')}
        $tag=(Get-Release).tag
        if ([IO.File]::ReadAllText($layout.Record) -notmatch [regex]::Escape($tag)) {$failures.Add("the install record does not name $tag")}
        foreach ($agent in $script:ExpectedAgents) {
            $entry=$null
            if ($record.agents) {$entry=$record.agents.$agent}
            if (-not $entry) {$failures.Add("the install record has no $agent entry");continue}
            if (-not $entry.last_check -or [string]$entry.last_check.result -cne 'pass') {$failures.Add("$agent did not pass its check")}
            $steps=@($entry.last_check.details | ForEach-Object {"$($_.step)=$($_.outcome)"}) -join ' '
            Write-Host "$agent check: $($entry.last_check.result) ($steps)"
        }
    }
    # TODO(8.2): with the real CLIs on PATH the agents' own commands must run,
    # so also require details step agent_command=pass (a local run without the
    # CLIs shows it as skipped), and one passing line per agent in install.out.
    $compare=Compare-Seed ($script:ManagedSeedFiles+$AllowChanged) $true
    foreach ($f in $compare) {$failures.Add($f)}
    if ($failures.Count) {foreach ($f in $failures) {Write-Host "FAIL: $f"};throw "assert-installed: $($failures.Count) failure(s)"}
    Write-Host 'assert-installed: OK'
}

# ---------------------------------------------------------- compare-seed ---
# Returns failure messages. Files outside the manifest (install record,
# backups, program, agent caches) are listed but are not failures.
function Compare-Seed([string[]]$Allowed,[bool]$AllowRegistry) {
    $expected=Read-SeedManifest $script:Manifest
    $failures=New-Object System.Collections.Generic.List[string]
    $same=0
    foreach ($rel in $expected.Keys) {
        $path=Join-Path $script:SeedHome $rel
        $allowedHere=@($Allowed | Where-Object {$_ -and ($rel -like $_)}).Count -gt 0
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            if ($allowedHere) {Write-Host "allowed-missing  $rel"} else {$failures.Add("seeded file missing: $rel")}
            continue
        }
        $hash=(Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($hash -ceq $expected[$rel]) {$same++;continue}
        if ($allowedHere) {Write-Host "allowed-changed  $rel"} else {$failures.Add("seeded file changed: $rel")}
    }
    if ($script:OnWindows -and (Test-Path -LiteralPath $script:RegistrySnapshot)) {
        $before=Read-Json $script:RegistrySnapshot
        $now=Get-UserPathSnapshot
        if ($before.exists -ne $now.exists -or $before.kind -cne $now.kind -or $before.value -cne $now.value) {
            if ($AllowRegistry) {Write-Host 'allowed-changed  HKCU\Environment\Path'} else {$failures.Add('HKCU\Environment\Path changed')}
        } else {$same++}
    }
    $extra=@(Get-SeedFiles $script:SeedHome | Where-Object {-not $expected.Contains($_)})
    Write-Host "compare-seed: $same unchanged, $($failures.Count) failure(s), $($extra.Count) file(s) not in the seed"
    foreach ($e in $extra) {Write-Host "  new: $e"}
    return ,$failures.ToArray()
}
function Invoke-CompareSeedPhase {
    $failures=Compare-Seed $AllowChanged $false
    if ($failures.Count) {foreach ($f in $failures) {Write-Host "FAIL: $f"};throw "compare-seed: $($failures.Count) failure(s)"}
    Write-Host 'compare-seed: byte-identical'
}

# ------------------------------------------------------------------ stop ---
function Invoke-StopPhase {
    $pidFile=Join-Path $WorkDir 'server.pid'
    if (Test-Path -LiteralPath $pidFile) {
        $id=[int]([IO.File]::ReadAllText($pidFile).Trim())
        $p=Get-Process -Id $id -ErrorAction SilentlyContinue
        if ($p) {Stop-Process -Id $id -Force;Write-Host "Stopped the server (pid $id)"}
        Remove-Item -LiteralPath $pidFile -Force
    }
    if ($script:OnWindows -and (Test-Path -LiteralPath $script:RegistrySnapshot)) {
        $before=Read-Json $script:RegistrySnapshot
        $now=Get-UserPathSnapshot
        if ($before.exists -ne $now.exists -or $before.kind -cne $now.kind -or $before.value -cne $now.value) {
            $key=[Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment',$true)
            try {
                if ($before.exists) {$key.SetValue('Path',[string]$before.value,[Microsoft.Win32.RegistryValueKind]$before.kind)}
                else {$key.DeleteValue('Path',$false)}
            } finally {$key.Close()}
            Write-Host 'Restored HKCU\Environment\Path to the snapshot taken by seed'
        }
    }
}

# ------------------------------------------------------- 8.2 scenarios ---
function Invoke-PendingScenario([string]$Name) {
    # TODO(8.2): menu path through a pseudo-terminal (script / ConPTY), rerun
    # ("Everything is current", byte-identical home), update from the previous
    # release, remove (seeded files byte-identical), cancel at Review, archive
    # 404, wrong hash, bad signature on update, already-invalid opencode.json.
    throw "Phase '$Name' is not implemented yet (reliable-installers task 8.2)"
}

# Phases that run the real installer change machine-wide state that a temp
# home cannot isolate (on Windows the user PATH lives in HKCU). They run only
# on a disposable CI runner, or when a maintainer explicitly opts in.
$machinePhases = @('install','menu-install','rerun','update','remove','cancel-at-review',
    'download-404','bad-hash','bad-signature','invalid-config')
if ($machinePhases -contains $Phase -and $env:GITHUB_ACTIONS -ne 'true' -and
    $env:MMHMEM_E2E_ALLOW_LOCAL -ne 'I understand this changes my user PATH') {
    throw ("Phase '$Phase' runs the real installer, which changes this machine's user PATH. " +
        "It runs only on GitHub Actions runners. To run it locally anyway, set " +
        "MMHMEM_E2E_ALLOW_LOCAL='I understand this changes my user PATH'.")
}

switch ($Phase) {
    'keygen' {Invoke-KeygenPhase}
    'release' {Invoke-ReleasePhase}
    'seed' {Invoke-SeedPhase}
    'serve' {Start-Server}
    'install' {Invoke-InstallPhase}
    'assert-installed' {Invoke-AssertInstalledPhase}
    'compare-seed' {Invoke-CompareSeedPhase}
    'stop' {Invoke-StopPhase}
    default {Invoke-PendingScenario $Phase}
}
# A phase that returns has passed. Native helpers (py, uname, zipsign) leave
# their own exit codes behind, and the Actions PowerShell wrappers exit with
# $LASTEXITCODE, so reset it here.
$global:LASTEXITCODE=0
