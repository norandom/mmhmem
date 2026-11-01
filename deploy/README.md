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

## 3. Connecting harnesses

See `deploy/skills/unified-memory/SKILL.md` (the portable agent playbook,
identical for local and serve modes) and `deploy/examples/` (Claude Code
CLI, Google Antigravity, OpenCode, Claude Desktop — each with a local
stdio variant and a serve-mode HTTP variant — plus agent-side OTLP export
to OpenObserve).
