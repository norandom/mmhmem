# mmhmem

<p align="center">
  <img src="assets/mmhmem.png" alt="Before and after mmhmem: fewer tokens sent to the credit card company" width="560">
</p>

Shared memory and file-reading tools for AI coding agents. Switch between
Claude Code, OpenCode, Codex, Claude Desktop and Antigravity without losing
your working state. Cache large-file summaries instead of paying to read the
same files again.

Each agent runs mmhmem locally and shares your per-user memory store. The
installers let you choose your agents and install the workflow playbook.

## Install

Installs the latest signed release. No checkout or Rust toolchain needed.

**Windows x64** (PowerShell 5.1 or 7):

```powershell
irm https://github.com/norandom/mmhmem/releases/latest/download/install.ps1 | iex
```

**Linux x86_64 and macOS** (Apple Silicon or Intel):

```sh
curl -fsSL https://github.com/norandom/mmhmem/releases/latest/download/install.sh | sh
```

The installer downloads the release archive for your computer, uses it only
if its SHA-256 matches the one built into the installer, and starts
`mmhmem setup`. Setup checks the release signature, then walks you through
three screens:

1. **Tools**: Claude Code, Codex, OpenCode, Claude Desktop and Antigravity.
   The ones found on your computer are ticked.
2. **Features** per tool: shared memory (required), the read-routing hook
   (on by default), a memory guide, a recap command and agent instructions.
3. **Review**: every file it will change, before anything is written. Each
   file is backed up first; press Enter to apply.

Setup then checks that each tool can start and use mmhmem, and lists what is
left for you to do:

- **Claude Code** and **OpenCode**: start a new session.
- **Codex**: start a new session and approve the mmhmem hook in `/hooks`.
  Codex skips hooks you have not approved.
- **Claude Desktop**: quit it completely and open it again.
- **Antigravity**: start a new session. Setup configures Antigravity itself:
  mmhmem's server entry in `~/.gemini/config/mcp_config.json` and the
  read-routing hook in `~/.gemini/config/hooks.json`, the files its CLI and
  IDE read.

The read-routing hook sends reads of 150 lines or more (whole files, or line
ranges that large) to `smart_read`. It needs a worker model; without one it stays inactive and setup says so.
See [worker settings](docs/configuration.md#worker-settings).

Optionally, check where an archive came from:
`gh attestation verify <archive> --repo norandom/mmhmem`.

## Update

```sh
mmhmem update
```

It shows the installed and the newest version, asks once, then downloads the
new release, checks its signature and checksum, and keeps your tools and
choices. If the new version needs to change files in your tools, it lists
them and asks before changing anything; otherwise it only checks your tools.
Start new agent sessions afterwards. When a newer release exists, agents see a
one-line note at session start; nothing installs itself
([update notice](docs/configuration.md#update-notice)).

`mmhmem setup` and the install command above offer the same update when
mmhmem is already set up. `mmhmem update` exists since v0.5.1: to update from
v0.5.0, run `mmhmem setup` and choose **Update to v0.5.1**.

## Change, check or remove

Run `mmhmem setup` and choose **Change agents and features**, **Check that
everything works** or **Remove mmhmem**. Removal takes out only what mmhmem
added, restores your files, and keeps your saved memories unless you tick
"Also delete my saved memories".

## Use it

Start a new agent session after installation. The playbook guides it to
resume your session, use cached summaries, search project memory and
checkpoint before you switch agents.

```sh
mmhmem stats          # last 7 days, rows with savings, fits the terminal
mmhmem stats --all    # every row and column
mmhmem stats --json   # all data for scripts
```

Memory and targeted file reads work without a worker model. For large-file
summaries and boilerplate generation, configure an OpenAI-compatible endpoint
such as Ollama, vLLM or a hosted provider. See [worker settings](docs/configuration.md#worker-settings).

Agents read files through `smart_read`; programs that already hold a
tool-result string (such as the mvibe proxy) call `shunt_content` for the
same cached summaries without a file path. The full tool list is in the
[tool reference](docs/usage.md#mcp-tools).

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

## Verifying routing adoption

Run `cargo test --test routing_adoption` for deterministic adapter tests. For
explicit live checks, set `E2E_LLM=1`, `E2E_CLAUDE_MODEL`, and
`E2E_OPENCODE_MODEL`, then run `cargo test --test routing_adoption -- --nocapture`.
On Windows npm installations, set `E2E_OPENCODE_BINARY` to the installed native
OpenCode executable when the command is a PowerShell wrapper. These runs use
synthetic projects and existing authentication; they make model requests.
An inconclusive cache identity or unavailable harness fails acceptance rather
than claiming savings. See `.kiro/specs/agent-tool-routing/validation.md` for
the current verified scope and remaining live prerequisites.
