#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
mkdir -p "$root/tmp"
scratch="$(mktemp -d "$root/tmp/codex-registration.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT
info() { :; }
# Match the installer's fatal-error contract even inside an if/subshell,
# where Bash disables errexit for ordinary failed commands.
die() { printf '%s\n' "$*" >&2; exit 1; }
source <(sed -n '/^CODEX_BEGIN=/,/^# want_agent/{ /^# want_agent/d; p; }' "$root/deploy/install_local.sh")
CODEX_CFG_DIR="$scratch"; cfg="$scratch/config.toml"; DRY_RUN=0; REMOVE=0
block="$(codex_block /bin/mmhmem)"
for layout in empty first last middle duplicate cross; do
  case "$layout" in
    empty) : > "$cfg" ;;
    first) printf '%s\n[other]\nx = 1\n' "$block" > "$cfg" ;;
    last) printf 'model = "preserved"\n%s' "$block" > "$cfg" ;;
    middle) printf 'model = "preserved"\n%s\n[other]\nx = 1\n' "$block" > "$cfg" ;;
    duplicate) printf '%s\n%s\n' "$block" "$block" > "$cfg" ;;
    cross) printf '%s\n' "${block/install_local.sh/install_local.ps1}" > "$cfg" ;;
  esac
  set_codex_registration /bin/mmhmem
  cp "$cfg" "$scratch/once"
  set_codex_registration /bin/mmhmem
  cmp "$cfg" "$scratch/once"
  [ "$(grep -c '^\[mcp_servers.mmhmem\]$' "$cfg")" -eq 1 ]
  [ -f "$cfg.mmhmem.bak" ]
  python3 - "$cfg" <<'PY'
import sys,tomllib
from pathlib import Path
config=tomllib.loads(Path(sys.argv[1]).read_text())
assert config['mcp_servers']['mmhmem']['command']=='/bin/mmhmem'
PY
  REMOVE=1; remove_codex_registration; REMOVE=0
  ! grep -q '^\[mcp_servers.mmhmem\]$' "$cfg"
done
for text in '[mcp_servers.mmhmem]' '[mcp_servers."mmhmem"]' "[mcp_servers.'mmhmem'.env]" "$CODEX_BEGIN" "$CODEX_END" 'mcp_servers = { mmhmem = {} }' $'[mcp_servers]\nmmhmem.command = "existing"'; do
  printf '%s' "$text" > "$cfg"; cp "$cfg" "$scratch/before"
  if (set_codex_registration /bin/mmhmem); then printf 'Ambiguous TOML accepted: %s\n' "$text" >&2; exit 1; fi
  cmp "$cfg" "$scratch/before"
done
printf '%s\n' 'PASS: Codex TOML parse, repeated/edge installs, cross-shell markers, removal and fail-closed config preservation'
