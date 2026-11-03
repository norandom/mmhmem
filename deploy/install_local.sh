#!/usr/bin/env bash
# mmhmem local-mode installer (macOS / Linux).
#
# Registers `mmhmem stdio` as a local MCP server in the harnesses you
# choose and installs the unified-memory playbook — or, with --remove,
# unregisters it again. No server, no token, no environment variables
# needed (local-mode Req 2.5). The per-user store is user data and is
# NEVER deleted by --remove; its location is printed instead.
#
# Usage:
#   ./install_local.sh                        # asks: scope, then harnesses
#   ./install_local.sh --global               # all projects, every detected harness
#   ./install_local.sh --global --agent claude,opencode
#   ./install_local.sh --project DIR [--agent LIST]
#   ./install_local.sh --remove [--global] [--agent LIST]
#   ./install_local.sh --binary PATH [--dry-run]
#
# Options:
#   --agent LIST   comma-separated harnesses: claude, opencode, desktop,
#                  antigravity, codex, all (default: interactive selection).
#                  Interactive mode asks for the list when this flag is absent.
#   --worker-model NAME   set the shunt worker model — e.g. an Ollama tag
#                  like qwen3:latest. Stored in .mmhmem.json (global or
#                  project per scope); interactive mode offers this when no
#                  flag is given.
#   --worker-url URL      worker endpoint (default is local Ollama at
#                  http://127.0.0.1:11434/v1/chat/completions).
#   --worker-api-key KEY  worker API key for hosted endpoints; stored in
#                  .mmhmem.json, never echoed.
#   --remove       uninstall: deregister the selected harnesses, remove the
#                  playbook copies and the copied binary. Store and audit
#                  files stay untouched.
#   --dry-run      print actions, change nothing.
#
# Behavior:
#   - Global installs copy the binary to ~/.local/bin (creating it) and
#     register the copy by absolute path, so GUI harnesses (Claude
#     Desktop) can spawn it without a shell PATH.
#   - Per-project installs reference the binary in place; Claude Desktop
#     is app-wide and is skipped in project mode.
#   - Claude Code CLI registers via its own CLI; OpenCode and Claude
#     Desktop configs are JSON-merged in place (python3 required for the
#     merge; without it the script prints the manual steps instead).
#   - Codex registers through ~/.codex/config.toml as a managed
#     [mcp_servers.mmhmem] block (global only; a project install skips it
#     with a note), installs $mmhmem-recap in ~/.agents/skills, and keeps
#     the legacy recap prompt in ~/.codex/prompts for compatibility.
#   - Antigravity imports MCP servers through its settings UI: the script
#     prints the ready-to-import snippet (and the removal step to undo it).
#   - Claude Code and OpenCode additionally get the /mmhmem-recap command
#     template copied into their command directory (scope-matched);
#     --remove removes it. Desktop and Antigravity have no user-defined
#     commands and get none.

set -euo pipefail

usage() { sed -n '2,52p' "$0" | sed 's/^# \{0,1\}//'; exit 0; }
die() { echo "install_local.sh: $*" >&2; exit 1; }
info() { echo "[mmhmem] $*"; }

GLOBAL=0
PROJECT_DIR=""
DRY_RUN=0
REMOVE=0
BIN_OVERRIDE=""
INSTALL_SOURCE=""
AGENTS=""
WORKER_MODEL=""
WORKER_URL=""
WORKER_KEY=""
while [ $# -gt 0 ]; do
  case "$1" in
    --global) GLOBAL=1 ;;
    --project)
      shift
      [ $# -ge 1 ] || die "--project needs a directory"
      PROJECT_DIR="$1"
      ;;
    --binary)
      shift
      [ $# -ge 1 ] || die "--binary needs a path"
      BIN_OVERRIDE="$1"
      ;;
    --install-source)
      shift
      [ $# -ge 1 ] || die '--install-source needs a path'
      INSTALL_SOURCE="$1"
      ;;
    --agent)
      shift
      [ $# -ge 1 ] || die "--agent needs a list (claude, opencode, desktop, antigravity, all)"
      AGENTS="$(printf '%s' "$1" | tr 'A-Z,' 'a-z,' | tr -d ' ')"
      ;;
    --worker-model)
      shift
      [ $# -ge 1 ] || die "--worker-model needs a model name"
      WORKER_MODEL="$1"
      ;;
    --worker-url)
      shift
      [ $# -ge 1 ] || die "--worker-url needs a URL"
      WORKER_URL="$1"
      ;;
    --worker-api-key)
      shift
      [ $# -ge 1 ] || die "--worker-api-key needs a key"
      WORKER_KEY="$1"
      ;;
    --remove) REMOVE=1 ;;
    --dry-run) DRY_RUN=1 ;;
    -h|--help) usage ;;
    *) die "unknown option $1 (see --help)" ;;
  esac
  shift
done
for a in ${AGENTS//,/ }; do
  case "$a" in
    claude|opencode|desktop|antigravity|codex|all) ;;
    *) die "unknown agent '$a' (valid: claude, opencode, desktop, antigravity, codex, all)" ;;
  esac
done

# --- scope -----------------------------------------------------------------
[ -n "$AGENTS" ] || [ -t 0 ] || die 'pass --agent with a comma-separated list, or --agent all (no terminal for selection)'
[ "$GLOBAL" -eq 1 ] && [ -n "$PROJECT_DIR" ] && die "--global and --project are mutually exclusive"
if [ -z "$PROJECT_DIR" ] && [ "$GLOBAL" -eq 0 ]; then
  [ -t 0 ] || die "pass --global or --project DIR (stdin is not a terminal)"
  while true; do
    printf 'Install mmhmem local mode:\n  [g] globally for all projects (user scope)\n  [p] for this project only (%s)\nChoose (g/p): ' "$PWD"
    read -r ans || die "no answer given"
    case "$ans" in
      g|G|global|GLOBAL|a|all) GLOBAL=1; break ;;
      p|P|project|PROJECT|t|this) PROJECT_DIR="$PWD"; break ;;
    esac
  done
fi
[ -z "$PROJECT_DIR" ] || [ -d "$PROJECT_DIR" ] || die "project dir not found: $PROJECT_DIR"

# --- worker settings (.mmhmem.json; corpus-warmup-prune Req 7.6) ------------
# worker_model is the activation switch: without it the worker stays off and
# shunting/warmup is disabled; the URL defaults to local Ollama, so an Ollama
# user only needs the model name here. Settings are stored in .mmhmem.json -
# the global one for user scope, the project one for project scope - and the
# binary reads them at startup; the harness registrations carry no worker env.
# The file is JSON-merged (other keys preserved); the API key is never echoed.
# Scope-selected worker-config file (project scope: the installer's project
# dir, which the spawned child also uses as its working directory).
WORKER_CONFIG_FILE=""
if [ "$REMOVE" -eq 0 ]; then
  if [ "$GLOBAL" -eq 1 ]; then
    WORKER_CONFIG_FILE="$HOME/.mmhmem.json"
  elif [ -n "$PROJECT_DIR" ]; then
    WORKER_CONFIG_FILE="$PROJECT_DIR/.mmhmem.json"
  fi
fi

# --- binary ----------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
worker_check() {
  MMHMEM_CHECK_MODEL="$WORKER_MODEL" MMHMEM_CHECK_API_URL="$WORKER_URL" MMHMEM_CHECK_API_KEY="$WORKER_KEY" \
    python3 "$SCRIPT_DIR/check_worker.py" "$1" "$WORKER_CONFIG_FILE"
}
if [ "$REMOVE" -eq 0 ] && command -v python3 >/dev/null 2>&1; then
  worker_check show || die 'cannot read worker settings'
  existing_model="$(worker_check model)" || die 'cannot read worker settings'
  existing_base="$(worker_check base)" || die 'cannot read worker settings'
  existing_key_label="$(worker_check key-label)" || die 'cannot read worker settings'
fi

if [ "$REMOVE" -eq 0 ]; then
  if [ -n "$BIN_OVERRIDE" ]; then
    BIN="$BIN_OVERRIDE"
  elif command -v mmhmem >/dev/null 2>&1; then
    BIN="$(command -v mmhmem)"
  elif [ -x "$REPO_ROOT/target/release/mmhmem" ]; then
    BIN="$REPO_ROOT/target/release/mmhmem"
  else
    die "mmhmem binary not found — build it (cargo build --release) or pass --binary PATH"
  fi
  if [ "$DRY_RUN" -eq 0 ] && [ -z "$INSTALL_SOURCE" ]; then
    BIN="$(cd "$(dirname "$BIN")" && pwd)/$(basename "$BIN")"
  fi
else
  # Removal only needs the standard copy location (global installs).
  BIN="$HOME/.local/bin/mmhmem"
fi

SKILL_SRC="$REPO_ROOT/deploy/skills/unified-memory/SKILL.md"
install_skill() {
  if [ "$DRY_RUN" -eq 1 ]; then
    info "[dry-run] copy SKILL.md -> $1/SKILL.md"
  else
    mkdir -p "$1"
    cp "${2:-$SKILL_SRC}" "$1/SKILL.md"
  fi
  info "playbook installed: $1/SKILL.md"
}
remove_skill() {
  if [ "$DRY_RUN" -eq 1 ]; then
    info "[dry-run] rm $1/SKILL.md"
    return
  fi
  rm -f "$1/SKILL.md"
  rmdir "$1" 2>/dev/null || true
  info "playbook removed: $1"
}
skill() {
  if [ "$REMOVE" -eq 1 ]; then remove_skill "$1"; else install_skill "$1"; fi
}

# /mmhmem-recap command templates (corpus-warmup-prune Req 9): copied per
# scope into the harness's own command directory; desktop/antigravity have
# no user-defined commands and get none.
COMMAND_SRC_CLAUDE="$REPO_ROOT/deploy/commands/claude/mmhmem-recap.md"
COMMAND_SRC_OPENCODE="$REPO_ROOT/deploy/commands/opencode/mmhmem-recap.md"
install_command() { # $1 = src, $2 = dst
  if [ "$DRY_RUN" -eq 1 ]; then
    info "[dry-run] copy $(basename "$1") -> $2"
    return
  fi
  mkdir -p "$(dirname "$2")"
  cp "$1" "$2"
  info "command installed: $2"
}
remove_command() { # $1 = dst
  if [ "$DRY_RUN" -eq 1 ]; then
    info "[dry-run] rm $1"
    return
  fi
  rm -f "$1"
  info "command removed: $1"
}
command_tmpl() { # $1 = src, $2 = dst
  if [ "$REMOVE" -eq 1 ]; then remove_command "$2"; else install_command "$1" "$2"; fi
}

# Merges one JSON assignment into a config file, preserving everything
# else in it. $1 = file, $2 = dotted path (a.b.c), $3 = JSON value,
# $4 = "secret" (optional) — a secret value is never echoed by the
# manual-merge fallback (Req 7.5: keys never appear in installer output).
json_merge() {
  cfg="$1"; path="$2"; value="$3"; secret="${4:-}"
  if [ "$DRY_RUN" -eq 1 ]; then
    info "[dry-run] merge $path into $cfg"
    MERGE_OK=1
    return 0
  fi
  mkdir -p "$(dirname "$cfg")"
  if command -v python3 >/dev/null 2>&1; then
    python3 - "$cfg" "$path" "$value" <<'PYEOF' && { MERGE_OK=1; return 0; }
import json, sys
cfg, path, value = sys.argv[1], sys.argv[2], json.loads(sys.argv[3])
try:
    with open(cfg) as f:
        data = json.load(f)
except (FileNotFoundError, json.JSONDecodeError):
    data = {}
node = data
parts = path.split(".")
for p in parts[:-1]:
    node = node.setdefault(p, {})
node[parts[-1]] = value
with open(cfg, "w") as f:
    json.dump(data, f, indent=2)
    f.write("\n")
PYEOF
  fi
  if [ "$secret" = "secret" ]; then
    value="<value withheld - secret; pass the flag again or edit the file by hand>"
  fi
  info "could not merge $cfg automatically — merge this by hand (deploy/examples/): $path = $value"
  MERGE_OK=0
  return 0
}
# Removes a JSON key from a config file. $1 = file, $2 = dotted path.
json_remove() {
  cfg="$1"; path="$2"
  if [ "$DRY_RUN" -eq 1 ]; then
    info "[dry-run] remove $path from $cfg"
    return 0
  fi
  [ -f "$cfg" ] || return 0
  if command -v python3 >/dev/null 2>&1; then
    python3 - "$cfg" "$path" <<'PYEOF' && return 0
import json, sys
cfg, path = sys.argv[1], sys.argv[2]
try:
    with open(cfg) as f:
        data = json.load(f)
except json.JSONDecodeError:
    sys.exit(0)
node = data
parts = path.split(".")
for p in parts[:-1]:
    if not isinstance(node, dict) or p not in node:
        sys.exit(0)
    node = node[p]
if isinstance(node, dict):
    node.pop(parts[-1], None)
with open(cfg, "w") as f:
    json.dump(data, f, indent=2)
    f.write("\n")
PYEOF
  fi
  info "could not update $cfg automatically — remove '$path' by hand"
  return 0
}

# JSON-quotes one scalar (plain model names/tags/URLs pass through) and
# writes the provided worker settings into the config file (JSON-merged,
# other keys preserved; only provided settings are touched).
json_quote() {
  q="${1//\\/\\\\}"
  printf '"%s"' "${q//\"/\\\"}"
}
write_worker_config() {
  [ -n "$WORKER_CONFIG_FILE" ] || return 0
  if [ -n "$WORKER_MODEL" ]; then
    json_merge "$WORKER_CONFIG_FILE" "worker_model" "$(json_quote "$WORKER_MODEL")"
  fi
  if [ -n "$WORKER_URL" ]; then
    json_merge "$WORKER_CONFIG_FILE" "worker_api_url" "$(json_quote "$WORKER_URL")"
  fi
  if [ -n "$WORKER_KEY" ]; then
    json_merge "$WORKER_CONFIG_FILE" "worker_api_key" "$(json_quote "$WORKER_KEY")" secret
  fi
}

# --- harness detection + selection ------------------------------------------
HAS_CLAUDE=0; command -v claude >/dev/null 2>&1 && HAS_CLAUDE=1
OPENCODE_GLOBAL_CFG="$HOME/.config/opencode/opencode.json"
HAS_OPENCODE=0
{ command -v opencode >/dev/null 2>&1 || [ -f "$OPENCODE_GLOBAL_CFG" ]; } && HAS_OPENCODE=1
DESKTOP_CFG_DIR=""
for d in "$HOME/Library/Application Support/Claude" "$HOME/.config/Claude"; do
  [ -d "$d" ] && DESKTOP_CFG_DIR="$d" && break
done
HAS_ANTIGRAVITY=0
{ command -v antigravity >/dev/null 2>&1 || [ -d "$HOME/.gemini" ]; } && HAS_ANTIGRAVITY=1
CODEX_CFG_DIR="$HOME/.codex"
HAS_CODEX=0
{ command -v codex >/dev/null 2>&1 || [ -d "$CODEX_CFG_DIR" ]; } && HAS_CODEX=1
CODEX_COMMAND_SRC="$REPO_ROOT/deploy/commands/codex/mmhmem-recap.md"

# Codex registers through ~/.codex/config.toml (global, TOML). The section
# is a managed block between marker comments so no TOML parser is needed:
# install replaces the block, remove strips it.
CODEX_BEGIN='# BEGIN mmhmem (managed by install_local.sh -- do not edit)'
CODEX_END='# END mmhmem'
codex_block() { # $1 = binary path (TOML-escaped)
  printf '%s\n[mcp_servers.mmhmem]\ncommand = "%s"\nargs = ["stdio"]\n%s' \
    "$CODEX_BEGIN" "$(printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g')" "$CODEX_END"
}
codex_unmanaged() {
  # Validate first; callers only replace config after awk succeeds.
  awk '
    /^# BEGIN mmhmem \(managed by install_local\.(ps1|sh) -- do not edit\)$/ {
      if (managed) { bad=1; exit 1 }; managed=1; next
    }
    /^# END mmhmem$/ { if (!managed) { bad=1; exit 1 }; managed=0; next }
    {
      if (index($0, "\"\"\"") || index($0, sprintf("%c%c%c",39,39,39))) { bad=1; exit 1 }
      if (managed) {
        if ($0 !~ /^[[:space:]]*(#.*|\[mcp_servers\.mmhmem\]|command[[:space:]]*=.*|args[[:space:]]*=.*)?[[:space:]]*$/) { bad=1; exit 1 }
        next
      }
      # Keep bracket escapes outside character classes: BusyBox awk and
      # Debian awk interpret escaped brackets inside a class differently.
      normal=$0
      gsub(/[[:space:]"]/, "", normal)
      gsub(sprintf("%c", 39), "", normal)
      gsub(/\[/, "", normal)
      gsub(/\]/, "", normal)
      if (normal ~ /^mcp_servers\.mmhmem([.#=]|$)/ || normal ~ /^mcp_servers=/ || normal ~ /^mmhmem([.#=]|$)/) { bad=1; exit 1 }
      print
    }
    END { if (bad || managed) exit 1 }
  ' "$1"
}
write_codex_registration() {
  cfg="$CODEX_CFG_DIR/config.toml"
  mkdir -p "$CODEX_CFG_DIR"
  temporary="$(mktemp "$cfg.XXXXXX")"
  if [ -f "$cfg" ] && ! codex_unmanaged "$cfg" > "$temporary"; then
    rm -f "$temporary"
    die 'Ambiguous/unmanaged mmhmem TOML needs manual review; Codex config unchanged.'
  fi
  if [ "$REMOVE" -eq 0 ]; then codex_block "$1" >> "$temporary"; printf '\n' >> "$temporary"; fi
  if [ -f "$cfg" ]; then cp -p "$cfg" "$cfg.mmhmem.bak"; chmod --reference="$cfg" "$temporary" 2>/dev/null || true; fi
  mv "$temporary" "$cfg"
}
set_codex_registration() {
  if [ "$DRY_RUN" -eq 1 ]; then info "[dry-run] merge [mcp_servers.mmhmem] into $CODEX_CFG_DIR/config.toml"; return; fi
  write_codex_registration "$1"
  info "codex registration written: $CODEX_CFG_DIR/config.toml"
}
remove_codex_registration() {
  if [ "$DRY_RUN" -eq 1 ]; then info "[dry-run] remove [mcp_servers.mmhmem] from $CODEX_CFG_DIR/config.toml"; return; fi
  [ -f "$CODEX_CFG_DIR/config.toml" ] || return 0
  write_codex_registration ''
  info "codex registration removed: $CODEX_CFG_DIR/config.toml"
}

# want_agent NAME: true when the selection (or the default) includes it.
want_agent() {
  [ -z "$AGENTS" ] && return 0
  case ",$AGENTS," in *",all,"*) return 0 ;; esac
  case ",$AGENTS," in *",$1,"*) return 0 ;; esac
  return 1
}
# agent_requested NAME: true only when the user named it explicitly.
agent_requested() {
  [ -z "$AGENTS" ] && return 1
  case ",$AGENTS," in *",all,"*) return 1 ;; esac
  want_agent "$1"
}

# Interactive harness selection when no --agent list was given.
if [ -z "$AGENTS" ] && [ -t 0 ]; then
  MENU_KEYS=""
  menu=""
  menu_count=0
  menu_add() { # $1 = key, $2 = description
    menu_count=$((menu_count + 1))
    MENU_KEYS="$MENU_KEYS $menu_count:$1"
    menu="${menu}  [$menu_count] $1 — $2"$'\n'
  }
  [ "$HAS_CLAUDE" -eq 1 ] && menu_add claude "Claude Code CLI"
  [ "$HAS_OPENCODE" -eq 1 ] && menu_add opencode "OpenCode"
  if [ -n "$DESKTOP_CFG_DIR" ] && [ "$GLOBAL" -eq 1 ]; then
    menu_add desktop "Claude Desktop"
  fi
  [ "$HAS_ANTIGRAVITY" -eq 1 ] && menu_add antigravity "print the import snippet"
  [ "$HAS_CODEX" -eq 1 ] && menu_add codex "OpenAI Codex CLI"
  [ "$menu_count" -gt 0 ] || die 'no supported agents detected; install an agent, then run this installer again'
  if [ "$menu_count" -gt 0 ]; then
    while true; do
      printf '\n  Select agents\n  ----------------------------------------\n%b  [a] All listed\n  [q] Cancel\n\n  Choose one or more (e.g. 1,3): ' "$menu"
      read -r ans || die "no answer given"
      ans="$(printf '%s' "$ans" | tr -d ' ')"
      [ -z "$ans" ] && continue
      case "$ans" in q|Q) exit 0 ;; a|A) AGENTS="all"; break ;; esac
      picked=""
      ok=1
      IFS=','
      for n in $ans; do
        key=""
        for pair in ${MENU_KEYS// /,}; do
          [ "${pair%%:*}" = "$n" ] && key="${pair#*:}"
        done
        if [ -z "$key" ]; then ok=0; break; fi
        picked="$picked,$key"
      done
      unset IFS
      [ "$ok" -eq 1 ] && [ -n "$picked" ] && AGENTS="${picked#,}" && break
      printf '  Please choose numbers from the list, a, or q.\n'
    done
  fi
fi

registered=""
missing_requested=""
note_missing() { # $1 = agent key, $2 = detected (1/0), $3 = doc name
  want_agent "$1" || return 0
  if [ "$2" -ne 1 ]; then
    if agent_requested "$1" && [ "$REMOVE" -eq 0 ]; then
      missing_requested="$missing_requested $1"
    else
      info "$3 not found on this machine — nothing to do for it"
    fi
  fi
}
note_missing claude "$HAS_CLAUDE" "Claude Code CLI"
note_missing opencode "$HAS_OPENCODE" "OpenCode"
note_missing desktop "$([ -n "$DESKTOP_CFG_DIR" ] && echo 1 || echo 0)" "Claude Desktop"
note_missing antigravity "$HAS_ANTIGRAVITY" "Antigravity"
note_missing codex "$HAS_CODEX" "Codex CLI"
[ -z "$missing_requested" ] || die "requested but not detected:$missing_requested"
# Refuse ambiguous Codex edits before committing binary or worker changes.
if [ "$GLOBAL" -eq 1 ] && want_agent codex && [ "$HAS_CODEX" -eq 1 ] && [ "$DRY_RUN" -eq 0 ] && [ -f "$CODEX_CFG_DIR/config.toml" ]; then
  codex_unmanaged "$CODEX_CFG_DIR/config.toml" > /dev/null || die 'Ambiguous/unmanaged mmhmem TOML needs manual review; Codex config unchanged.'
fi

# Plan every selected instruction destination before any integration writes.
ROUTING_AGENTS=()
if want_agent claude && [ "$HAS_CLAUDE" -eq 1 ]; then ROUTING_AGENTS+=(claude); fi
if want_agent opencode && [ "$HAS_OPENCODE" -eq 1 ]; then ROUTING_AGENTS+=(opencode); fi
ROUTING_PLANS='[]'
if [ "${#ROUTING_AGENTS[@]}" -gt 0 ]; then
  command -v python3 >/dev/null 2>&1 || die 'Python 3 is required for safe routing onboarding'
  routing_args=(--home "$HOME" --project "${PROJECT_DIR:-$PWD}" --agents "${ROUTING_AGENTS[@]}")
  [ "$GLOBAL" -eq 0 ] || routing_args+=(--global-scope)
  [ "$REMOVE" -eq 0 ] || routing_args+=(--remove)
  ROUTING_PLANS="$(python3 "$SCRIPT_DIR/routing_onboarding.py" prepare "${routing_args[@]}")" || die 'Routing preflight failed; no integrations were changed'
fi

# --- optional worker setup -------------------------------------------------
if [ "$REMOVE" -eq 0 ] && [ -z "$WORKER_MODEL" ] && [ -t 0 ]; then
  printf '\n  Optional: summary cache\n  ----------------------------------------\n'
  printf '  A worker model prepares reusable file summaries.\n  Memory works without it.\n'
  printf '  Keep existing settings by answering N.\n  This does not download or start a model.\n'
  printf '  Configure a worker model now? [y/N]: '
  read -r configure_worker || die 'no answer given'
  case "$configure_worker" in
    y|Y|yes|YES)
      printf '  Model ID [%s] (Enter to keep): ' "${existing_model:-not set}"
      read -r WORKER_MODEL || die 'no answer given'
      [ -n "$WORKER_MODEL" ] || WORKER_MODEL="${existing_model:-}"
      if [ -n "$WORKER_MODEL" ]; then
        printf '  Press Enter to keep each bracketed value.\n'
        printf '  OpenAI-compatible base URL [%s]: ' "${existing_base:-http://127.0.0.1:11434/v1}"
        read -r entered_url || die 'no answer given'
        if [ -n "$entered_url" ]; then
          entered_url="${entered_url%/}"
          case "$entered_url" in
            */chat/completions) WORKER_URL="$entered_url" ;;
            *) WORKER_URL="$entered_url/chat/completions" ;;
          esac
        fi
        if [ -z "$WORKER_KEY" ]; then
          printf '  Empty = keep existing key; local Ollama needs no key.\n'
          printf '  API key [%s] (hidden): ' "${existing_key_label:-not set}"
          read -r -s WORKER_KEY || die 'no answer given'
          printf '\n'
        fi
      fi
      ;;
  esac
fi
# --- end optional worker setup ---------------------------------------------
if [ "$REMOVE" -eq 0 ]; then
  if [ "$DRY_RUN" -eq 1 ]; then
    info '[dry-run] worker connection check skipped (no API request)'
  elif command -v python3 >/dev/null 2>&1; then
    worker_check check || die 'worker validation failed; installation unchanged'
  elif [ -n "$WORKER_MODEL$WORKER_URL$WORKER_KEY${WORKER_API_KEY:-}" ] || [ -f "$WORKER_CONFIG_FILE" ]; then
    die 'Python 3 is required to validate worker settings; install it and retry'
  fi
fi
if [ -n "$INSTALL_SOURCE" ] && [ "$REMOVE" -eq 0 ] && [ "$DRY_RUN" -eq 0 ]; then
  mkdir -p "$(dirname "$BIN")"
  staged_bin="$(mktemp "$(dirname "$BIN")/.mmhmem.XXXXXX")"
  trap 'rm -f "$staged_bin"' EXIT
  cp "$INSTALL_SOURCE" "$staged_bin"
  chmod +x "$staged_bin"
  if [ -f "$BIN" ]; then cp -p "$BIN" "$BIN.old"; fi
  mv -f "$staged_bin" "$BIN"
fi

# --- worker config file (before registrations; they stay worker-env-free) ---
# Req 7.6: the installer writes .mmhmem.json instead of embedding worker env.
WORKER_CONFIG_WRITTEN=0
if [ "$REMOVE" -eq 0 ] && [ -n "$WORKER_CONFIG_FILE" ]; then
  if [ -n "$WORKER_MODEL" ] || [ -n "$WORKER_URL" ] || [ -n "$WORKER_KEY" ]; then
    if [ "$DRY_RUN" -eq 1 ]; then
      write_worker_config
      info "[dry-run] worker settings would be stored in $WORKER_CONFIG_FILE (provided settings written; other keys preserved)"
      WORKER_CONFIG_WRITTEN=1
    else
      MERGE_OK=1
      write_worker_config
      if [ "$MERGE_OK" -eq 1 ]; then
        info "worker settings stored in $WORKER_CONFIG_FILE (provided settings written; other keys preserved)"
        WORKER_CONFIG_WRITTEN=1
      else
        info "worker settings NOT stored in $WORKER_CONFIG_FILE - merge by hand as printed above (python3 is required for automatic merge)"
      fi
    fi
  fi
fi

if [ "${#ROUTING_AGENTS[@]}" -gt 0 ]; then
  routing_commit_args=()
  [ "$DRY_RUN" -eq 0 ] || routing_commit_args+=(--dry-run)
  printf '%s' "$ROUTING_PLANS" | python3 "$SCRIPT_DIR/routing_onboarding.py" commit "${routing_commit_args[@]}" || die 'Routing commit failed; earlier operations may have completed'
fi

# --- Claude Code CLI ---------------------------------------------------------
if want_agent claude && [ "$HAS_CLAUDE" -eq 1 ]; then
  if [ "$REMOVE" -eq 1 ]; then
    if [ "$GLOBAL" -eq 1 ]; then
      if [ "$DRY_RUN" -eq 1 ]; then
        info "[dry-run] claude mcp remove mmhmem -s user"
      else
        claude mcp remove mmhmem -s user >/dev/null 2>&1 || info "no global mmhmem registration in Claude Code"
      fi
    elif [ -n "$PROJECT_DIR" ]; then
      if [ "$DRY_RUN" -eq 1 ]; then
        info "[dry-run] claude mcp remove mmhmem (in $PROJECT_DIR)"
      else
        (cd "$PROJECT_DIR" && claude mcp remove mmhmem >/dev/null 2>&1) || info "no project mmhmem registration in $PROJECT_DIR"
      fi
    fi
    skill "$HOME/.claude/skills/unified-memory"
    if [ "$GLOBAL" -eq 1 ]; then
      command_tmpl "$COMMAND_SRC_CLAUDE" "$HOME/.claude/commands/mmhmem-recap.md"
    else
      command_tmpl "$COMMAND_SRC_CLAUDE" "$PROJECT_DIR/.claude/commands/mmhmem-recap.md"
    fi
    registered="claude-code $registered"
  else
    if [ "$GLOBAL" -eq 1 ]; then
      [ "$DRY_RUN" -eq 0 ] && claude mcp remove mmhmem -s user >/dev/null 2>&1 || true
      if [ "$DRY_RUN" -eq 1 ]; then
        info "[dry-run] claude mcp add --transport stdio --scope user mmhmem -- $BIN stdio"
      else
        claude mcp add --transport stdio --scope user mmhmem -- "$BIN" stdio
      fi
    else
      (
        cd "$PROJECT_DIR"
        [ "$DRY_RUN" -eq 0 ] && claude mcp remove mmhmem >/dev/null 2>&1 || true
        if [ "$DRY_RUN" -eq 1 ]; then
          info "[dry-run] claude mcp add --transport stdio mmhmem -- $BIN stdio (in $PROJECT_DIR)"
        else
          claude mcp add --transport stdio mmhmem -- "$BIN" stdio
        fi
      )
    fi
    skill "$HOME/.claude/skills/unified-memory"
    registered="claude-code $registered"
    if [ "$GLOBAL" -eq 1 ]; then
      command_tmpl "$COMMAND_SRC_CLAUDE" "$HOME/.claude/commands/mmhmem-recap.md"
    else
      command_tmpl "$COMMAND_SRC_CLAUDE" "$PROJECT_DIR/.claude/commands/mmhmem-recap.md"
    fi
  fi
fi

# --- OpenCode -----------------------------------------------------------------
if want_agent opencode && [ "$HAS_OPENCODE" -eq 1 ]; then
  CFG="$OPENCODE_GLOBAL_CFG"
  [ "$GLOBAL" -eq 1 ] || CFG="$PROJECT_DIR/opencode.json"
  if [ "$REMOVE" -eq 1 ]; then
    json_remove "$CFG" "mcp.servers.mmhmem"
  else
    json_merge "$CFG" "mcp.servers.mmhmem" \
      "{\"type\": \"local\", \"command\": [$(json_quote "$BIN"), \"stdio\"]}"
  fi
  if [ "$GLOBAL" -eq 1 ]; then
    skill "$HOME/.config/opencode/skills/unified-memory"
    command_tmpl "$COMMAND_SRC_OPENCODE" "$HOME/.config/opencode/command/mmhmem-recap.md"
  else
    command_tmpl "$COMMAND_SRC_OPENCODE" "$PROJECT_DIR/.opencode/command/mmhmem-recap.md"
  fi
  registered="opencode $registered"
fi

# --- Claude Desktop (global scope only) ----------------------------------------
if want_agent desktop && [ -n "$DESKTOP_CFG_DIR" ]; then
  if [ "$GLOBAL" -eq 1 ]; then
    if [ "$REMOVE" -eq 1 ]; then
      json_remove "$DESKTOP_CFG_DIR/claude_desktop_config.json" "mcpServers.mmhmem"
    else
      json_merge "$DESKTOP_CFG_DIR/claude_desktop_config.json" "mcpServers.mmhmem" \
        "{\"command\": $(json_quote "$BIN"), \"args\": [\"stdio\"]}"
    fi
    case "$DESKTOP_CFG_DIR" in
      *Application*) skill "$HOME/Library/Claude/skills/unified-memory" ;;
      *) skill "$HOME/.config/Claude/skills/unified-memory" ;;
    esac
    registered="claude-desktop $registered"
  elif [ "$REMOVE" -eq 0 ]; then
    info "Claude Desktop is app-wide; skipped for a per-project install (run again with --global)"
  fi
fi

# --- Antigravity (manual import; no public config path) --------------------------
if want_agent antigravity && [ "$HAS_ANTIGRAVITY" -eq 1 ]; then
  if [ "$REMOVE" -eq 1 ]; then
    info "Antigravity: remove the mmhmem server in Antigravity Settings → MCP (added manually)"
  else
    info "Antigravity: add a stdio MCP server in Settings with this JSON:"
    echo "  { \"mcpServers\": { \"mmhmem\": { \"command\": $(json_quote "$BIN"), \"args\": [\"stdio\"] } } }"
  fi
  registered="antigravity-manual $registered"
fi

# --- OpenAI Codex CLI (global config.toml; managed block) ---------------------------
if want_agent codex && [ "$HAS_CODEX" -eq 1 ]; then
  if [ "$REMOVE" -eq 1 ]; then
    remove_codex_registration
    remove_command "$CODEX_CFG_DIR/prompts/mmhmem-recap.md"
    remove_skill "$HOME/.agents/skills/unified-memory"
    remove_skill "$HOME/.agents/skills/mmhmem-recap"
    registered="codex $registered"
  elif [ "$GLOBAL" -eq 1 ]; then
    set_codex_registration "$BIN"
    install_skill "$HOME/.agents/skills/unified-memory"
    install_skill "$HOME/.agents/skills/mmhmem-recap" "$REPO_ROOT/deploy/skills/mmhmem-recap/SKILL.md"
    command_tmpl "$CODEX_COMMAND_SRC" "$CODEX_CFG_DIR/prompts/mmhmem-recap.md"
    registered="codex $registered"
  else
    info "Codex reads a global config only; skipped for a per-project install (run again with --global)"
  fi
fi

# --- copied binary (global installs) ----------------------------------------------
if [ "$GLOBAL" -eq 1 ]; then
  USER_BIN="$HOME/.local/bin"
  if [ "$REMOVE" -eq 1 ]; then
    if [ "$AGENTS" != all ]; then
      info 'shared binary kept for other agents; use --remove --agent all to remove it'
    elif [ -f "$USER_BIN/mmhmem" ]; then
      if [ "$DRY_RUN" -eq 1 ]; then
        info "[dry-run] rm $USER_BIN/mmhmem"
      else
        if rm -f "$USER_BIN/mmhmem"; then
          info "removed binary copy: $USER_BIN/mmhmem"
        else
          info "binary copy is in use (a harness may be running): $USER_BIN/mmhmem - delete it manually later"
        fi
      fi
    fi
  elif [ "$BIN" != "$USER_BIN/mmhmem" ]; then
    if [ "$DRY_RUN" -eq 1 ]; then
      info "[dry-run] cp $BIN -> $USER_BIN/mmhmem"
    else
      mkdir -p "$USER_BIN"
      staged_bin="$(mktemp "$USER_BIN/.mmhmem.XXXXXX")"
      trap 'rm -f "$staged_bin"' EXIT
      cp "$BIN" "$staged_bin"
      chmod +x "$staged_bin"
      if [ -f "$USER_BIN/mmhmem" ]; then cp -p "$USER_BIN/mmhmem" "$USER_BIN/mmhmem.old"; fi
      mv -f "$staged_bin" "$USER_BIN/mmhmem"
    fi
  fi
  BIN="$USER_BIN/mmhmem"
  if [ "$REMOVE" -eq 0 ]; then
    case ":$PATH:" in
      *":$USER_BIN:"*) ;;
      *)
        case "${SHELL:-}" in
          */zsh) path_profile="$HOME/.zshrc" ;;
          */bash) path_profile="$HOME/.bashrc" ;;
          *) path_profile="$HOME/.profile" ;;
        esac
        path_line='export PATH="$HOME/.local/bin:$PATH" # mmhmem user tools'
        if [ "$DRY_RUN" -eq 1 ]; then
          info "[dry-run] add ~/.local/bin to PATH in $path_profile"
        else
          if ! grep -Fqx "$path_line" "$path_profile" 2>/dev/null; then
            printf '\n%s\n' "$path_line" >> "$path_profile"
          fi
          info "PATH configured in $path_profile"
          info 'Open a new terminal to run mmhmem by name.'
        fi
        ;;
    esac
  fi
fi

# --- summary ----------------------------------------------------------------------
[ -n "$registered" ] || [ "$WORKER_CONFIG_WRITTEN" -eq 1 ] || die "nothing to do: no harness selected/detected"
MODE="project: $PROJECT_DIR"
[ "$GLOBAL" -eq 1 ] && MODE="global (all projects)"
echo
if [ "$REMOVE" -eq 1 ]; then
  info "mmhmem local mode removed ($MODE)"
  info "harnesses: ${registered% }"
  info "per-user store left intact (it is your data): ~/.local/share/mmhmem or ~/Library/Application Support/mmhmem — delete it manually if you really want"
  info ".mmhmem.json kept (it may hold your worker settings and API key)"
else
  info "mmhmem local mode installed ($MODE)"
  info "harnesses: ${registered% }"
  info "binary: $BIN"
  if [ -n "$WORKER_MODEL" ]; then
    if [ "$WORKER_CONFIG_WRITTEN" -eq 1 ]; then
      info "worker model: $WORKER_MODEL (shunting and warmup active; stored in $WORKER_CONFIG_FILE)"
    else
      info "worker model: $WORKER_MODEL (store it in $WORKER_CONFIG_FILE by hand — the automatic merge failed as printed above)"
    fi
  else
    info "worker settings unchanged; summaries and warmup require worker_model in $WORKER_CONFIG_FILE or WORKER_MODEL in the environment"
  fi
  info "store: per-user default (~/.local/share/mmhmem or ~/Library/Application Support/mmhmem); created on first use"
  info "check your savings any time: $BIN stats"
fi
if [ "$DRY_RUN" -eq 1 ]; then
  info "dry run — nothing was written; re-run without --dry-run to apply"
fi
if [ "$REMOVE" -eq 0 ] && [ -t 0 ]; then
  printf '\n  Optional: explore and prebuild your project cache\n'
  printf '  Show the prompt to run in your agent? [y/N]: '
  read -r warmup || die 'no answer given'
  case "$warmup" in
    y|Y|yes|YES)
      printf '  Open the project in your agent.\n  Restart its mmhmem connection to load new settings.\n'
      printf '  A reachable worker model is required.\n  Remote providers may charge for summaries.\n'
      printf '\n  Paste this prompt into your agent:\n\n'
      printf '  Explore this repository and identify its key source files.\n'
      printf '  Use mmhmem shunt_warmup to prebuild their summary cache\n'
      printf '  in batches of up to 25 files.\n  Report what was cached or skipped.\n'
      ;;
  esac
fi
