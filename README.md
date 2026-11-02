# mmhmem

<p align="center">
  <img src="assets/mmhmem.jpg" alt="mmhmem motto" width="560">
</p>

Shared memory and file-reading tools for AI coding agents. Switch between
Claude Code, OpenCode, Codex, Claude Desktop and Antigravity without losing
your working state. Cache large-file summaries instead of paying to read the
same files again.

Each agent runs mmhmem locally and shares your per-user memory store. The
installers register detected agents and install the workflow playbook.

## Install

Download and install the latest release with no checkout or Rust toolchain.

### Linux x86_64

```sh
curl -fsSL https://raw.githubusercontent.com/norandom/mmhmem/HEAD/install.sh | bash
```

Linux release builds default to static musl for broad distribution compatibility.
The current published `v0.2.2` predates that default and still requires glibc;
see [Linux compatibility](docs/installation.md#linux-compatibility).

### Windows x64

Run in PowerShell 5.1 or later:

```powershell
irm https://raw.githubusercontent.com/norandom/mmhmem/HEAD/install.ps1 | iex
```

Close agents using mmhmem before updating a running Windows binary.

### macOS — Apple Silicon or Intel

```sh
curl -fsSL https://raw.githubusercontent.com/norandom/mmhmem/HEAD/install.sh | bash
```

The installer selects the matching architecture automatically.

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
