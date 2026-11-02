# Installation

The [README](../README.md#install) has the commands for each platform. The
bootstraps download the binary from the latest GitHub release and installer
companions from the repository's default branch. Re-running updates the
binary while preserving memory and worker settings unless you explicitly
provide new settings.

Windows updates can run while agents remain open. The installer renames the
old executable and puts the new version at the registered path. Existing
processes keep their current version until restarted; new processes use the
updated version. Old executables still in use are retained and cleaned up by
a later install after those processes exit.

| Platform | Release asset | Binary location | Memory location |
|---|---|---|---|
| Linux x86_64 | `mmhmem-linux-x86_64` | `~/.local/bin/mmhmem` | `~/.local/share/mmhmem` |
| macOS Apple Silicon | `mmhmem-macos-aarch64` | `~/.local/bin/mmhmem` | `~/Library/Application Support/mmhmem` |
| macOS Intel | `mmhmem-macos-x86_64` | `~/.local/bin/mmhmem` | `~/Library/Application Support/mmhmem` |
| Windows x64 | `mmhmem.exe` | `%LOCALAPPDATA%\Programs\mmhmem\mmhmem.exe` | `%APPDATA%\mmhmem` |

The Unix bootstrap needs Bash, curl or wget and standard Unix utilities.
Python 3 enables automatic JSON configuration merges; jq is not required.
Add the binary directory to your shell's PATH to run `mmhmem` yourself;
agent registrations use its absolute path. Linux ARM64 has no release asset.
On Windows use the native installer; WSL is only a CI build environment.

## Linux compatibility

The default Linux release build is static musl. Its declared support floor
is Linux 5.10 on baseline x86-64/SSE2, without mandatory AVX. CI inspects ELF
linkage and tests one executable on Debian 12 and Alpine 3.22. Container tests
share a host kernel; they do not certify every CPU or the oldest supported
kernel. Each verified bundle records its actual kernel, CPU and image digests.

The currently published [v0.2.2 asset](https://github.com/norandom/mmhmem/releases/tag/v0.2.2)
still uses glibc. The latest-release download gains static portability only
after the verified candidate is published and its checksum confirmed. See the
[release procedure](../deploy/README.md#4-verified-linux-release-transition).

## Installer options

On Linux or macOS, append arguments to the bootstrap:

```sh
curl -fsSL https://raw.githubusercontent.com/norandom/mmhmem/HEAD/install.sh | bash -s -- --agent claude
curl -fsSL https://raw.githubusercontent.com/norandom/mmhmem/HEAD/install.sh | bash -s -- --dry-run
curl -fsSL https://raw.githubusercontent.com/norandom/mmhmem/HEAD/install.sh | bash -s -- --remove
```

On Windows, save the script to pass options:

```powershell
irm https://raw.githubusercontent.com/norandom/mmhmem/HEAD/install.ps1 -OutFile install.ps1
.\install.ps1 -Agent codex
.\install.ps1 -DryRun
.\install.ps1 -Remove
```

Bootstraps default to user-wide registration for detected agents. Removal
keeps the memory store. Dry runs leave installed binaries and configuration
unchanged; temporary companion downloads are cleaned up. A source checkout
can instead run `deploy/install_local.sh` or `deploy/install_local.ps1` with
an explicit binary; see [development](development.md).

## Agent connections and plugins

For manual local registrations, use [Claude Code's stdio command](../deploy/examples/claude-code.md),
[OpenCode](../deploy/examples/opencode-local.json), [Antigravity](../deploy/examples/antigravity-mcp_config-local.json)
or the local section of [Claude Desktop](../deploy/examples/claude-desktop.md).
The [playbook guide](../deploy/skills/unified-memory/README.md) lists per-agent paths.

The bootstraps install a standalone playbook. To manage it as a plugin instead:

```sh
claude plugin marketplace add https://github.com/norandom/mmhmem.git
claude plugin install unified-memory@mmhmem
codex plugin marketplace add norandom/mmhmem
codex plugin add unified-memory@mmhmem
```

The plugin contains the skill, not a binary. Remove only the standalone
`unified-memory` directory from `~/.claude/skills` or `~/.agents/skills` to
avoid duplicate discovery, then restart the agent. See the
[marketplace guide](../.marketplace/README.md) for other supported manifests.
