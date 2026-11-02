#Requires -Version 5.1
<#
.SYNOPSIS
    Run the mmhmem Dagger CI pipeline from the Windows dev host (task 14.1).

.DESCRIPTION
    The Dagger module (ci/) must run on the WSL build VM's native filesystem:
    dagger's source filesync off the DrvFS mount (/mnt/d) hits 9p metadata
    cache conflicts ("size changed during sync") and crawling target/ once
    killed the engine. This script rsyncs the repo (minus build/VCS artifacts)
    into the Debian WSL home and runs dagger there — same module, same
    entrypoints as GitHub Actions (.github/workflows/ci.yml); only the
    transport differs (dev-environment steering).

.EXAMPLE
    powershell -NoProfile -File scripts\dagger-ci.ps1              # full pipeline
    powershell -NoProfile -File scripts\dagger-ci.ps1 -Call fmt   # one function
#>
param(
    # Dagger function to call (default: all — fmt, clippy default+otel, tests,
    # docker build, compose config, deploy artifacts, tokei report).
    [string]$Call = "all",
    # Extra dagger call arguments (e.g. --src for an explicit directory).
    [string[]]$ExtraArgs = @()
)

$ErrorActionPreference = 'Stop'
$repo = Join-Path $PSScriptRoot '..'

# 1. Sync sources into the WSL build VM (delete-stray so removed files vanish).
wsl -d Debian -e bash -c "rsync -a --delete --exclude target --exclude tmp --exclude .openidea --exclude .git --exclude data --exclude log --exclude workspace --exclude .env --exclude .kiro --exclude .opencode /mnt/d/Source/mmhmem/ ~/mmhmem-ci/"
if ($LASTEXITCODE -ne 0) { throw "rsync into the WSL build VM failed" }

# 2. Run the dagger module from the synced copy (--src .. = the synced repo
#    root; it has no target/.git — the excludes above guarantee that).
#    dagger streams its progress trace on stderr: under Stop preference PS 5.1
#    would abort on the first stderr line, so relax for the native call and
#    judge success purely by the exit code.
$argList = @('-d', 'Debian', '-e', 'bash', '-c',
    "cd ~/mmhmem-ci/ci && ~/.local/bin/dagger call $Call --src .. $($ExtraArgs -join ' ')")
$ErrorActionPreference = 'Continue'
& wsl @argList
$code = $LASTEXITCODE
$ErrorActionPreference = 'Stop'
exit $code
