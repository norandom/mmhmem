# mmhmem

<p align="center">
  <img src="assets/mmhmem.jpg" alt="mmhmem motto" width="560">
</p>

Shared memory and file-reading tools for AI coding agents. Switch between
Claude Code, OpenCode, Codex, Claude Desktop and Antigravity without losing
your working state. Cache large-file summaries instead of paying to read the
same files again.

Each agent runs mmhmem locally and shares your per-user memory store. The
installers let you choose your agents and install the workflow playbook.

## Install

Download and install the latest release with no checkout or Rust toolchain.

### Linux x86_64

```sh
curl -fsSL https://raw.githubusercontent.com/norandom/mmhmem/HEAD/install.sh | bash
```

Linux release builds default to static musl for broad distribution compatibility.
The current published `v0.2.2` predates that default and still requires glibc;
see [Linux compatibility](docs/installation.md#linux-compatibility).

The menu offers **Install/update** or **Remove**, then lets you choose agents
by number (for example `1,3`). Use `a` for all listed agents or `q` to cancel.
To open removal directly:

```sh
curl -fsSL https://raw.githubusercontent.com/norandom/mmhmem/HEAD/install.sh | bash -s -- --remove
```

For unattended use, append `--agent codex,claude` or `--agent all`.
Removal preserves your memory and worker settings.

### Windows x64

Run in PowerShell 5.1 or later:

```powershell
irm https://raw.githubusercontent.com/norandom/mmhmem/HEAD/install.ps1 | iex
```

The menu offers **Install/update** or **Remove**, then lets you choose agents
by number (for example `1,3`). Use `a` for all listed agents or `q` to cancel.
Agents can stay open during updates; restart them when ready to use the new version.

To open removal directly, or select agents without menus:

```powershell
$installer = [scriptblock]::Create((irm https://raw.githubusercontent.com/norandom/mmhmem/HEAD/install.ps1))
& $installer -Remove                     # choose agents to remove
# Or: & $installer -Agent 'codex,claude' # install/update only these agents
# Or: & $installer -Remove -Agent all   # remove all detected integrations
```

Removal preserves your memory and worker settings. Claude Desktop detection
supports both traditional and MSIX installations.

Selected Claude Code installations add owned guidance to `.claude/rules/mmhmem.md`
(global or project scope). OpenCode uses global `.config/opencode/AGENTS.md` or
project `AGENTS.md`. Existing text is preserved; changed files retain one
`.mmhmem.bak` backup. Removal strips only the owned section. Start a fresh agent
session after updating. Unsupported configuration-root overrides or ambiguous
instruction ownership stop onboarding before integrations are changed; use the
default configuration or resolve the reported conflict first.

### macOS — Apple Silicon or Intel

```sh
curl -fsSL https://raw.githubusercontent.com/norandom/mmhmem/HEAD/install.sh | bash
```

The installer selects the matching architecture automatically.

The menu offers **Install/update** or **Remove**, then lets you choose agents
by number (for example `1,3`). Use `a` for all listed agents or `q` to cancel.
To open removal directly:

```sh
curl -fsSL https://raw.githubusercontent.com/norandom/mmhmem/HEAD/install.sh | bash -s -- --remove
```

For unattended use, append `--agent codex,claude` or `--agent all`.
Removal preserves your memory and worker settings.

For install locations, agent selection, dry runs, updates and removal, see
[installation options](docs/installation.md).

## Use it

Restart your agent after installation. The playbook guides it to resume your
session, use cached summaries, search project memory and checkpoint before
you switch agents.

```sh
mmhmem stats
mmhmem stats --json
```

Memory and targeted file reads work without a worker model. For large-file
summaries and boilerplate generation, configure an OpenAI-compatible endpoint
such as Ollama, vLLM or a hosted provider. See [worker settings](docs/configuration.md#worker-settings).

## Optional server

Use one shared Linux or Docker deployment when agents on different machines
need the same memory store. Clients on every platform use the same remote
connection setup. See the [server guide](docs/server.md).

## Documentation

- [Installation, agent connections and marketplace plugins](docs/installation.md)
- [Configuration](docs/configuration.md)
- [Daily use and tool reference](docs/usage.md)
- [Development, CI and releases](docs/development.md)
- [Concept and background](docs/concept.md)

Unix onboarding adds owned routing instructions for selected Claude Code/OpenCode agents.
Global destinations are `~/.claude/rules/mmhmem.md` and
`~/.config/opencode/AGENTS.md`; project destinations are `.claude/rules/mmhmem.md`
and `AGENTS.md`. Existing user text is preserved with one `.mmhmem.bak` backup.
Use `--remove` to remove owned guidance or `--dry-run` to validate without writing.
Start a fresh harness session after updates. Custom configuration roots or
instruction-disable controls require manual review; the installer reports their
variable names without exposing configuration contents.
# Verifying routing adoption

Run `cargo test --test routing_adoption` for deterministic adapter tests. For
explicit live checks, set `E2E_LLM=1`, `E2E_CLAUDE_MODEL`, and
`E2E_OPENCODE_MODEL`, then run `cargo test --test routing_adoption -- --nocapture`.
On Windows npm installations, set `E2E_OPENCODE_BINARY` to the installed native
OpenCode executable when the command is a PowerShell wrapper. These runs use
synthetic projects and existing authentication; they make model requests.
An inconclusive cache identity or unavailable harness fails acceptance rather
than claiming savings. See `.kiro/specs/agent-tool-routing/validation.md` for
the current verified scope and remaining live prerequisites.
