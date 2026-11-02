# Deploying mmhmem

mmhmem is one self-hosted binary with env-only configuration. The local
layer needs no deployment at all — the harness spawns `mmhmem stdio` per
session against a per-user store (see the README "Quickstart: local mode").
This page covers the SERVER deployment shapes: systemd on a Linux host
(default) or Docker Compose (repo root `Dockerfile` + `docker-compose.yml`,
task 12.3). The server is the optional upgrade for cross-machine access,
central audit, and multi-user bearer auth.

Server-required env vars (fail-fast at startup): `AGENT_MEM_TOKEN`,
`AGENT_MEM_ALLOWED_ROOTS` (comma-separated). Everything else has defaults —
see `.env.example` at the repo root for the full list.

The daemon serves HTTP. The administrator owns any reverse proxy/load
balancer and TLS termination; their configuration is outside this guide.

## 1. systemd + logrotate (Linux host)

```sh
# one-time system user + dirs
sudo useradd --system --home-dir /var/lib/agent-memory --create-home \
     --shell /usr/sbin/nologin agent-memory
sudo install -d -o agent-memory -g agent-memory /var/log/agent-memory

# install
sudo install -d -m 0750 /etc/mmhmem
sudo cp deploy/examples/mmhmem.env /etc/mmhmem/mmhmem.env   # then edit it
sudo cp target/release/mmhmem /usr/local/bin/mmhmem
sudo cp deploy/systemd/mmhmem.service /etc/systemd/system/mmhmem.service
sudo cp deploy/logrotate.d/agent-memory /etc/logrotate.d/agent-memory
sudo systemctl daemon-reload && sudo systemctl enable --now mmhmem
```

Notes:

- `target/release/mmhmem` above is a native source build. For the portable
  Linux release, substitute the verified bundle's `mmhmem-linux-x86_64` file
  from section 4; a plain `cargo build --release` is not the musl release gate.
- The unit hardens the daemon (`ProtectSystem=strict`); every
  `AGENT_MEM_ALLOWED_ROOTS` entry needs a matching commented-out
  `#ReadWritePaths=` line uncommented in the unit — the daemon writes there
  when `boilerplate_write` runs.
- `logrotate` rotates `/var/log/agent-memory/audit.jsonl` daily, keeps 90
  compressed days at 0640, and never needs to signal the daemon (the audit
  writer opens append-per-write, Req 7.4). The optional OpenObserve batch ship
  of rotated archives is a commented `postrotate` block in the config.
- Verify a unit on any Linux box: `systemd-analyze verify mmhmem.service`;
  parse-check logrotate with `logrotate --debug /etc/logrotate.d/agent-memory`.

## 2. Docker Compose

```sh
cp .env.example .env          # then edit it
docker compose up -d --wait   # healthcheck hits /healthz
```

The compose file bind-mounts the data dir, the audit log dir (host-side
logrotate owns rotation, Req 8.7), and the allowed workspace roots (read-only;
mount `rw` if you want `boilerplate_write` to write inside the container).
Its OTel-enabled Dockerfile remains a GNU build; it is not migrated by the
plain Linux static-release work.

## 3. Connecting harnesses

See `deploy/skills/unified-memory/SKILL.md` (the portable agent playbook,
identical for local and serve modes) and `deploy/examples/` (Claude Code
CLI, Google Antigravity, OpenCode, Claude Desktop — each with a local
stdio variant and a serve-mode HTTP variant — plus agent-side OTLP export
to OpenObserve).

## 4. Verified Linux release transition

### Support and evidence

The plain Linux candidate is built with Rust 1.98.1 for
`x86_64-unknown-linux-musl`, using baseline x86-64/SSE2 and a Linux 5.10
support policy. No host-specific CPU or mandatory AVX flags are used. CI
inspects the ELF for dynamic loaders, shared-library requirements and GLIBC
versions, then runs the same executable on Debian 12 and Alpine 3.22. It also
checks startup in scratch and pristine distribution images before adding
test instrumentation.

The recorded local validation used kernel `6.18.54.1-microsoft-standard-WSL2`;
hosted run [41495286781](https://github.com/norandom/mmhmem/actions/runs/41495286781)
recorded `7.0.0-1017-azure`. In each run both containers shared that host
kernel. Neither run certifies Linux 5.10 or every baseline CPU; use the
bundle's `verification.json` for its exact tested kernel, CPU and image
digests. The WSL kernel entry describes CI infrastructure, not an installation
target. User installation targets remain ordinary Linux, macOS and native
Windows.

The Unix bootstrap needs Bash, curl or wget and standard Unix utilities.
Python 3 enables automatic JSON config merges; without it the installer
prints manual merge instructions. jq is used by CI checks, not required by
the bootstrap. Rust, a compiler and a checkout are unnecessary for installing
a release asset. DNS configuration and filesystem permissions still apply.
Existing HTTPS worker trust uses the distribution trust store and rejects
untrusted connections; trust administration is unchanged. No TLS-specific
tests or proxy/load-balancer setup are part of this release gate.

### Build and export for review

From the repository root on the Windows development host, use native
PowerShell and choose a fresh output directory beneath `tmp/`:

```powershell
# Full canonical quality pipeline, then export its same verified candidate:
powershell -NoProfile -File scripts/dagger-ci.ps1 -Call all -ExportPath tmp/linux-release-review

# Or run just the static release gate and export, in a separate fresh directory:
powershell -NoProfile -File scripts/dagger-ci.ps1 -Call static-release -ExportPath tmp/linux-static-review
```

The wrapper captures the native Git revision and worktree cleanliness before
source sync and supplies a fresh validation ID. It rejects existing export
directories and paths outside repository scratch, including reparse-point
escapes. Dirty local trees can be tested, but their bundles are marked
`publishable: false`. The local export is for review; the publisher accepts
only an artifact downloaded from the identified hosted CI run. Linux build
and execution stay inside Dagger containers through this entrypoint; do not
run ad hoc validation in WSL or use Git Bash.

The bundle contains `mmhmem-linux-x86_64`, `SHA256SUMS`, `verification.json`
and `reports/`. No stripping, rebuilding or other executable modification is
allowed after verification. The File-returning `release()` consumer selects
this same verified executable.

### Validate and deliver to an existing draft

Choose a commit with a successful `CI` workflow run whose required Dagger,
Windows and macOS jobs all passed. The release tag must resolve to that exact
commit. Prepare the tag and an existing release draft through the normal
maintainer process; the publisher neither creates nor publishes a release.

Hosted CI exports the bundle as
`linux-static-<full-commit>-<run_id>-<run_attempt>` with validation ID
`ci-<run_id>-<run_attempt>`. Keep that artifact and its reports with the release
record. Set the existing tag and matching run ID, then use PowerShell 5.1+
with authenticated `gh`:

```powershell
$tag = Read-Host 'Existing draft release tag'
$runId = Read-Host 'Successful CI run ID for that tag commit'
powershell -NoProfile -File scripts/publish-linux-release.ps1 -Tag $tag -RunId $runId -ValidateOnly
powershell -NoProfile -File scripts/publish-linux-release.ps1 -Tag $tag -RunId $runId
```

Validation checks repository/workflow/run/tag identity, required job results,
clean publishable source, checksums and report consistency before upload. The
delivery uploads only the verified Linux executable, `SHA256SUMS` and
`verification.json`, then downloads the draft assets and compares their
hashes. An identical asset is skipped; a conflicting asset fails without
overwrite. A retry checks remote state after an interrupted upload. No
arbitrary local binary is accepted, and the release remains a draft.

The release owner must also supply the existing other-platform assets for
the same tag: `mmhmem.exe`, `mmhmem-macos-aarch64` and `mmhmem-macos-x86_64`.
Their build mechanics and marketplace paths are unchanged. Publish the
complete release manually only after draft verification has succeeded.

### Confirm latest, then update the portability claim

The latest downloadable Linux asset was still the dynamic v0.2.2 build when
checked on 2026-11-02: SHA-256
`309a88e752cac0adf0e2c4d17c0bb5991dc091b67e10011342918c7af902ab7d`,
with interpreter `/lib64/ld-linux-x86-64.so.2`. Keep the README's glibc caveat
until the complete static release has been published and the latest endpoint
serves the verified bytes.

After publication, download the CI artifact from the selected successful run
and extract it beneath repository `tmp/linux-ci-bundle/`. Compare the latest
endpoint with that manifest; do not substitute a checksum from a local build:

```powershell
$expected = (Get-Content -Raw tmp/linux-ci-bundle/verification.json | ConvertFrom-Json).artifact.sha256
$latest = 'tmp/latest-linux-confirmation'
Invoke-WebRequest -UseBasicParsing -Uri 'https://github.com/norandom/mmhmem/releases/latest/download/mmhmem-linux-x86_64' -OutFile $latest
$actual = (Get-FileHash -LiteralPath $latest -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actual -cne $expected) { throw 'Latest Linux asset differs from the verified candidate' }
```

Only after that comparison succeeds, replace the README's current glibc-only
paragraph and transition notice with this prepared text, and update the
steering release-status note to identify the published release:

> The Linux x86_64 release is statically linked with musl and requires no
> dynamic loader or shared libraries. The same executable is verified on
> Debian 12 and Alpine 3.22. Its support floor is Linux 5.10 and baseline
> x86-64/SSE2; the verification report records the actual tested kernel and
> CPU. Bash, curl or wget and standard Unix utilities are still needed for
> the installer; Python 3 enables automatic JSON config merges.

Publication-dependent acceptance remains open until the latest-download
checksum and this documentation update are complete.

### Rollback

If validation fails, retain the current release. If a published candidate
regresses, stop its rollout and issue a corrective release or use the
maintainer process to restore the previous latest-release designation.
Confirm that the latest endpoint serves the selected known-good checksum and
restore the matching compatibility guidance. Preserve affected memory stores,
worker settings and harness registrations. Never rebuild or overwrite the
tested assets in place to make an existing release appear repaired.
