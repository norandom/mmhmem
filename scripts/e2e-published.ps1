#Requires -Version 5.1
<#
.SYNOPSIS
The published-release install (reliable-installers task 9.2), dot-sourced by
scripts/e2e-run.ps1 and run by .github/workflows/published-install.yml.
.DESCRIPTION
The installer e2e (installer-e2e.yml) proves the bootstraps it renders for a
local server. These phases prove what users actually run: the README
one-liners against github.com, in the same seeded, isolated home, checked by
the same assert-installed. Evidence only, never a merge or release gate.

  published-release  -Tag <latest tag> (the workflow looks it up with
                     `gh api repos/norandom/mmhmem/releases/latest`):
                     download that tag's SHA256SUMS, SHA256SUMS.sig and this
                     platform's archive; SHA256SUMS.sig must verify with a key
                     in src/setup/release_keys.rs, the archive must match its
                     SHA256SUMS line and its own signature; the program inside
                     is hashed. Writes site/release.json (role 'published'),
                     which assert-installed reads for the tag and the hash.
                     Downloads only; installs and changes nothing.
  published-install  the README one-liner, exactly as written, without a
                     terminal, in the isolated environment of home/:
                       irm https://github.com/norandom/mmhmem/releases/latest/download/install.ps1 | iex
                       curl -fsSL https://github.com/norandom/mmhmem/releases/latest/download/install.sh | sh
  assert-published   the bootstrap installed <tag>; each of Claude Code,
                     Codex and OpenCode recorded with exactly the default
                     features (memory and the read-routing hook); then
                     assert-installed -Run published-install (exit 0, record
                     names <tag>, program = the verified archive's, every
                     agent's own check OK, seeded files, no leftover processes)
#>

$script:PublishedRepository='norandom/mmhmem'
$script:PublishedLatest="https://github.com/$script:PublishedRepository/releases/latest/download"
$script:PublishedDownload="https://github.com/$script:PublishedRepository/releases/download"
# What a no-terminal install selects for every agent (flow_select.rs select_defaults).
$script:PublishedDefaultFeatures='mcp,read_hook'

# The README one-liner for this OS, byte for byte (tests/install_docs.rs pins
# the README; scripts/test-e2e-harness.ps1 checks this text against it).
function Get-PublishedOneLinerText([bool]$Windows) {
    if ($Windows) {return "irm $script:PublishedLatest/install.ps1 | iex"}
    return "curl -fsSL $script:PublishedLatest/install.sh | sh"
}
# Program and arguments that run it without a terminal, as Get-OneLiner does
# for the local release: Windows in this step's shell (powershell.exe 5.1 or
# pwsh) plus `exit $LASTEXITCODE` (install.ps1 never calls exit); Unix under
# setsid where it exists, so the bootstrap's /dev/tty probe fails.
function Get-PublishedOneLiner([bool]$Windows,[string]$Shell,[bool]$Setsid) {
    $line=Get-PublishedOneLinerText $Windows
    if ($Windows) {return @{File=$Shell;Arguments="-NoProfile -NonInteractive -Command `"$line; exit `$LASTEXITCODE`""}}
    if ($Setsid) {return @{File='setsid';Arguments="-w /bin/sh -c `"$line`""}}
    return @{File='/bin/sh';Arguments="-c `"$line`""}
}

# The SHA-256 SHA256SUMS lists for one asset; exactly one line must name it.
function Get-ChecksumEntry([string]$Text,[string]$Name) {
    $found=@(foreach ($line in ($Text -split "`n")) {
        if ($line -cmatch '^([0-9a-f]{64})  (\S+)\r?$' -and $Matches[2] -ceq $Name) {$Matches[1]}
    })
    if ($found.Count -ne 1) {throw "SHA256SUMS lists $Name $($found.Count) time(s), expected once"}
    return $found[0]
}
function ConvertFrom-HexText([string]$Hex) {
    if ($Hex -cnotmatch '^([0-9a-f]{2})+$') {throw 'Not lowercase hex'}
    $bytes=New-Object byte[] ($Hex.Length/2)
    for ($i=0;$i -lt $bytes.Length;$i++) {$bytes[$i]=[Convert]::ToByte($Hex.Substring(2*$i,2),16)}
    return ,$bytes
}
# Unpacks the one entry of a release archive with the tools the bootstraps
# use (Expand-Archive for the Windows zip, tar for the Unix tar.gz; both read
# the zipsign signature the archive carries). Returns the entry's path.
function Expand-ReleaseEntry([string]$Archive,[string]$Entry,[string]$Destination) {
    New-Item -ItemType Directory -Path $Destination -Force | Out-Null
    if ($Archive -like '*.zip') {
        Expand-Archive -LiteralPath $Archive -DestinationPath $Destination -Force
    } else {
        & tar -xzf $Archive -C $Destination $Entry
        if ($LASTEXITCODE -ne 0) {throw "tar could not unpack $Entry from $Archive"}
    }
    $path=Join-Path $Destination $Entry
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {throw "$Archive does not contain $Entry"}
    return $path
}
function Save-PublishedAsset([string]$Url,[string]$Path) {
    # Windows PowerShell 5.1 may default to protocols GitHub no longer accepts.
    try {[Net.ServicePointManager]::SecurityProtocol=[Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12} catch {}
    $ProgressPreference='SilentlyContinue'   # the progress bar slows 5.1 downloads down a lot
    for ($try=1;;$try++) {
        try {Invoke-WebRequest -UseBasicParsing -Uri $Url -OutFile $Path -TimeoutSec 120;return}
        catch {if ($try -ge 3) {throw "Download failed: $Url ($($_.Exception.Message))"};Start-Sleep -Seconds (5*$try)}
    }
}

# ----------------------------------------------------- published-release ---
function Invoke-PublishedReleasePhase {
    if (-not $Tag) {throw '-Tag is required (the latest release: gh api repos/norandom/mmhmem/releases/latest)'}
    Assert-ArchiveTag $Tag
    Assert-ZipsignVersion
    $target=@(Get-ReleaseArchiveTargets $Tag | Where-Object {$_.Key -ceq (Get-PlatformKey)})[0]
    $folder="published-$Tag"
    $dir=Join-Path $script:Site $folder
    if (Test-Path -LiteralPath $dir) {Remove-Item -LiteralPath $dir -Recurse -Force}
    New-Item -ItemType Directory -Path $dir -Force | Out-Null
    $files=[ordered]@{}
    foreach ($name in @('SHA256SUMS','SHA256SUMS.sig',$target.Archive)) {
        $path=Join-Path $dir $name
        Save-PublishedAsset "$script:PublishedDownload/$Tag/$name" $path
        $files[$name]=(Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
        Write-Host "Downloaded $name ($($files[$name]))"
    }
    $sums=Join-Path $dir 'SHA256SUMS';$archive=Join-Path $dir $target.Archive
    # The key setup itself trusts; never one from the download.
    $keys=@(Get-EmbeddedReleaseKeys)
    if (-not $keys.Count) {throw 'src/setup/release_keys.rs lists no release key'}
    $publicHex=$null;$publicFile=$null
    foreach ($k in $keys) {
        $candidate=Join-Path $dir "key-$($k.Substring(0,16)).pub"
        [IO.File]::WriteAllBytes($candidate,(ConvertFrom-HexText $k))
        if (Test-ReleaseChecksumSignature $sums (Join-Path $dir 'SHA256SUMS.sig') $candidate) {$publicHex=$k;$publicFile=$candidate;break}
    }
    if (-not $publicHex) {throw "SHA256SUMS.sig of $Tag does not verify with any key in src/setup/release_keys.rs"}
    Write-Host "SHA256SUMS.sig verifies with release key $publicHex"
    $listed=Get-ChecksumEntry ([IO.File]::ReadAllText($sums)) $target.Archive
    if ($listed -cne $files[$target.Archive]) {throw "$($target.Archive) is $($files[$target.Archive]), SHA256SUMS lists $listed"}
    if (-not (Test-ReleaseArchiveSignature $archive $publicFile)) {throw "$($target.Archive) does not carry a valid signature by that key"}
    Write-Host "$($target.Archive) matches SHA256SUMS and its signature verifies"
    $program=Expand-ReleaseEntry $archive $target.Entry (Join-Path $dir 'unpacked')
    $binaries=[ordered]@{}
    $binaries[$target.Key]=(Get-FileHash -LiteralPath $program -Algorithm SHA256).Hash.ToLowerInvariant()
    Write-Json (Join-Path $script:Site 'release.json') ([ordered]@{tag=$Tag;role='published';folder=$folder
        base_url=$script:PublishedLatest;public_key=$publicHex;binaries=$binaries;files=$files})
    Write-Host "Published $Tag for $($target.Key): program SHA-256 $($binaries[$target.Key])"
}

# ----------------------------------------------------- published-install ---
function Invoke-PublishedInstallPhase {
    $release=Get-Release
    if ([string]$release.role -cne 'published') {throw 'site/release.json is not the published release (run published-release first)'}
    $command=Get-PublishedOneLiner $script:OnWindows $(if ($script:OnWindows) {Get-ShellPath} else {''}) ([bool](Get-Command setsid -ErrorAction SilentlyContinue))
    Write-Host "Running the published one-liner: $(Get-PublishedOneLinerText $script:OnWindows)"
    $code=Invoke-Isolated 'published-install' $command.File $command.Arguments
    if ($code -ne 0) {throw "The published one-liner exited with $code"}
}

# ---------------------------------------------------- assert-published ---
function Invoke-AssertPublishedPhase {
    $release=Get-Release
    if ([string]$release.role -cne 'published') {throw 'site/release.json is not the published release (run published-release first)'}
    $failures=New-Object Collections.Generic.List[string]
    # Both bootstraps print "Installing mmhmem <tag> for <target>." first.
    if (-not (Read-Log 'published-install').Contains("Installing mmhmem $($release.tag) for ")) {
        $failures.Add("the published one-liner did not install $($release.tag) (a newer release since published-release?)")
    }
    $record=$null
    $paths=Get-InstalledPaths
    if (Test-Path -LiteralPath $paths.Record -PathType Leaf) {$record=Read-Json $paths.Record}
    if ($record) {
        $choices=Get-AgentChoices $record
        foreach ($agent in $script:ExpectedAgents) {
            if ([string]$choices[$agent] -cne $script:PublishedDefaultFeatures) {
                $failures.Add("$agent was recorded with features '$($choices[$agent])', expected the defaults '$script:PublishedDefaultFeatures'")
            }
        }
    }
    foreach ($f in $failures) {Write-Host "FAIL: $f"}
    # Exit code, record, program hash, checks, seeded files, processes.
    $script:Run='published-install'
    $installed=$null
    try {Invoke-AssertInstalledPhase} catch {$installed=$_.Exception.Message}
    if ($failures.Count -or $installed) {throw "assert-published: $($failures.Count) failure(s)$(if ($installed) {"; $installed"})"}
    Write-Host "assert-published ($($release.tag)): OK"
}
