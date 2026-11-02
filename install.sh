#!/usr/bin/env bash
# mmhmem one-line installer for Linux and macOS — pipe it to bash:
#
#   curl -fsSL https://raw.githubusercontent.com/norandom/mmhmem/HEAD/install.sh | bash
#
# A thin bootstrap around deploy/install_local.sh (the engine): it detects
# the platform, downloads the latest release binary to ~/.local/bin/mmhmem,
# fetches the engine plus its companion files (the unified-memory playbook
# and the /mmhmem-recap command templates) from the default branch, and runs
# the engine with your flags. No checkout, no Rust toolchain.
#
# Deliberate refusals (project rules):
#   - WSL is a CI build environment, never an install target: on Windows
#     use install.ps1 with the mmhmem.exe release asset.
#   - Linux aarch64 has no release asset yet: build from source.

set -euo pipefail

die() { echo "install.sh: $*" >&2; exit 1; }
info() { echo "[mmhmem] $*"; }

RAW="https://raw.githubusercontent.com/norandom/mmhmem/HEAD"
REL="https://github.com/norandom/mmhmem/releases/latest/download"

usage() {
  cat <<'EOF'
mmhmem one-line installer (Linux / macOS) — latest release, no checkout:

  curl -fsSL https://raw.githubusercontent.com/norandom/mmhmem/HEAD/install.sh | bash

All arguments pass through to deploy/install_local.sh, e.g.:

  ... | bash -s -- --global --agent claude      # one harness, user-wide
  ... | bash -s -- --global --agent codex      # codex via ~/.codex/config.toml
  ... | bash -s -- --remove --global          # uninstall (the store is kept)

With no arguments, menus select install/update or removal and which agents
to configure globally. Use --agent LIST for unattended runs.
A worker model stays unconfigured (shunting off) until you pass
--worker-model NAME or edit .mmhmem.json.
EOF
  exit 0
}

# --- flags (parsed first so --help needs no platform) -------------------------
REMOVE=0
DRY_RUN=0
HAS_SCOPE=0
HAS_AGENT=0
for a in "$@"; do
  case "$a" in
    -h|--help) usage ;;
    --remove)  REMOVE=1 ;;
    --dry-run) DRY_RUN=1 ;;
    --global|--project) HAS_SCOPE=1 ;;
    --agent) HAS_AGENT=1 ;;
  esac
done
if [ "$HAS_SCOPE" -eq 0 ]; then
  set -- --global "$@"
fi

# The bootstrap itself may arrive on stdin via curl | bash. Read menus from
# the controlling terminal, never from the downloaded script stream.
if [ "$HAS_AGENT" -eq 0 ]; then
  { exec 3<> /dev/tty; } 2>/dev/null || die 'agent selection needs a terminal; pass --agent codex (or --agent all) for unattended use'
  if [ -z "${NO_COLOR:-}" ] && [ "${TERM:-dumb}" != dumb ]; then
    if command -v curl >/dev/null 2>&1; then
      curl -fsSL "$RAW/deploy/banner.ansi" >&3 2>/dev/null || true
    elif command -v wget >/dev/null 2>&1; then
      wget -qO- "$RAW/deploy/banner.ansi" >&3 2>/dev/null || true
    fi
  fi
  printf '\n  mmhmem  /  shared memory for your agents\n  ----------------------------------------\n' >&3
  if [ "$REMOVE" -eq 0 ]; then
    while true; do
      printf '\n  [1] Install or update\n  [2] Remove integrations\n  [q] Cancel\n\n  Choose an action: ' >&3
      read -r choice <&3 || die 'no answer given'
      case "$choice" in
        1) break ;;
        2) REMOVE=1; set -- "$@" --remove; break ;;
        q|Q) exit 0 ;;
      esac
    done
  fi
fi

# --- platform -----------------------------------------------------------------
os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
  Linux*)
    if [ -r /proc/version ] && grep -qi microsoft /proc/version; then
      die "this is WSL - a CI build environment, not an install target; on Windows use install.ps1 (README quickstart)"
    fi
    [ "$arch" = "x86_64" ] || die "no Linux $arch release asset yet - build from source: cargo build --release"
    asset="mmhmem-linux-x86_64"
    ;;
  Darwin*)
    case "$arch" in
      arm64)  asset="mmhmem-macos-aarch64" ;;
      x86_64) asset="mmhmem-macos-x86_64" ;;
      *)      die "no macOS $arch release asset" ;;
    esac
    ;;
  MINGW*|MSYS*|CYGWIN*)
    die "Windows detected - use the PowerShell installer: install.ps1 (README quickstart)"
    ;;
  *) die "unsupported platform: $os ($arch)" ;;
esac

# --- download helper -----------------------------------------------------------
fetch() { # $1 = url, $2 = output path
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL -o "$2" "$1" || die "download failed: $1"
  elif command -v wget >/dev/null 2>&1; then
    wget -q --output-document="$2" "$1" || die "download failed: $1"
  else
    die "need curl or wget to download mmhmem"
  fi
}

# Stage downloads before touching an existing installation.
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
BIN="$HOME/.local/bin/mmhmem"
if [ "$REMOVE" -eq 0 ] && [ "$DRY_RUN" -eq 0 ]; then
  info "downloading the latest release binary ($asset)"
  fetch "$REL/$asset" "$tmp/mmhmem"
  [ -s "$tmp/mmhmem" ] || die "empty binary download"
  chmod +x "$tmp/mmhmem"
elif [ "$REMOVE" -eq 0 ]; then
  info "[dry-run] download $REL/$asset -> $BIN"
fi

# --- engine + companion files (raw fetch, no checkout) -------------------------
tree="$tmp/deploy"
mkdir -p "$tree/skills/unified-memory" "$tree/commands/claude" \
         "$tree/commands/opencode" "$tree/commands/codex"
fetch "$RAW/deploy/install_local.sh" "$tree/install_local.sh"
if [ "$REMOVE" -eq 0 ]; then
  fetch "$RAW/deploy/check_worker.py" "$tree/check_worker.py"
  fetch "$RAW/deploy/skills/unified-memory/SKILL.md"    "$tree/skills/unified-memory/SKILL.md"
  fetch "$RAW/deploy/commands/claude/mmhmem-recap.md"   "$tree/commands/claude/mmhmem-recap.md"
  fetch "$RAW/deploy/commands/opencode/mmhmem-recap.md" "$tree/commands/opencode/mmhmem-recap.md"
  fetch "$RAW/deploy/commands/codex/mmhmem-recap.md"    "$tree/commands/codex/mmhmem-recap.md"
fi

# --- run the engine -------------------------------------------------------------
if [ "$REMOVE" -eq 0 ] && [ "$DRY_RUN" -eq 0 ]; then
  set -- "$@" --install-source "$tmp/mmhmem"
fi
info "running the installer"
if [ "$HAS_AGENT" -eq 0 ]; then
  bash "$tree/install_local.sh" --binary "$BIN" "$@" <&3
else
  bash "$tree/install_local.sh" --binary "$BIN" "$@" < /dev/null
fi
