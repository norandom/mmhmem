#!/usr/bin/env bash
# mmhmem local-mode installer (macOS / Linux / WSL).
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
#                  antigravity, all (default: all detected). Interactive
#                  mode asks for the list when this flag is absent.
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
#   - Antigravity imports MCP servers through its settings UI: the script
#     prints the ready-to-import snippet (and the removal step to undo it).

set -euo pipefail

usage() { sed -n '2,31p' "$0" | sed 's/^# \{0,1\}//'; exit 0; }
die() { echo "install_local.sh: $*" >&2; exit 1; }
info() { echo "[mmhmem] $*"; }

GLOBAL=0
PROJECT_DIR=""
DRY_RUN=0
REMOVE=0
BIN_OVERRIDE=""
AGENTS=""
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
    --agent)
      shift
      [ $# -ge 1 ] || die "--agent needs a list (claude, opencode, desktop, antigravity, all)"
      AGENTS="$(printf '%s' "$1" | tr 'A-Z,' 'a-z,' | tr -d ' ')"
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
    claude|opencode|desktop|antigravity|all) ;;
    *) die "unknown agent '$a' (valid: claude, opencode, desktop, antigravity, all)" ;;
  esac
done

# --- scope -----------------------------------------------------------------
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

# --- binary ----------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
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
  BIN="$(cd "$(dirname "$BIN")" && pwd)/$(basename "$BIN")"
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
    cp "$SKILL_SRC" "$1/SKILL.md"
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

# Merges one JSON assignment into a config file, preserving everything
# else in it. $1 = file, $2 = dotted path (a.b.c), $3 = JSON value.
json_merge() {
  cfg="$1"; path="$2"; value="$3"
  if [ "$DRY_RUN" -eq 1 ]; then
    info "[dry-run] merge $path into $cfg"
    return 0
  fi
  mkdir -p "$(dirname "$cfg")"
  if command -v python3 >/dev/null 2>&1; then
    python3 - "$cfg" "$path" "$value" <<'PYEOF' && return 0
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
  info "could not merge $cfg automatically — merge this by hand (deploy/examples/): $path = $value"
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
  want_agent "$1"
}

# Interactive harness selection when no --agent list was given.
if [ -z "$AGENTS" ] && [ -t 0 ] && [ -t 1 ]; then
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
  if [ "$menu_count" -gt 0 ]; then
    while true; do
      printf 'Which harness(es)?\n%b  [a] all listed\nChoose (comma-separated numbers, or a): ' "$menu"
      read -r ans || die "no answer given"
      ans="$(printf '%s' "$ans" | tr -d ' ')"
      [ -z "$ans" ] && continue
      if [ "$ans" = "a" ] || [ "$ans" = "A" ]; then AGENTS=""; break; fi
      picked=""
      ok=1
      IFS=','
      for n in $ans; do
        key=""
        for pair in $MENU_KEYS; do
          [ "${pair%%:*}" = "$n" ] && key="${pair#*:}"
        done
        if [ -z "$key" ]; then ok=0; break; fi
        picked="$picked,$key"
      done
      unset IFS
      [ "$ok" -eq 1 ] && [ -n "$picked" ] && AGENTS="${picked#,}" && break
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
[ -z "$missing_requested" ] || die "requested but not detected:$missing_requested"

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
      "{\"type\": \"local\", \"command\": [\"$BIN\", \"stdio\"]}"
  fi
  if [ "$GLOBAL" -eq 1 ]; then
    skill "$HOME/.config/opencode/skills/unified-memory"
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
        "{\"command\": \"$BIN\", \"args\": [\"stdio\"]}"
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
    echo "  { \"mcpServers\": { \"mmhmem\": { \"command\": \"$BIN\", \"args\": [\"stdio\"] } } }"
  fi
  registered="antigravity-manual $registered"
fi

# --- copied binary (global installs) ----------------------------------------------
if [ "$GLOBAL" -eq 1 ]; then
  USER_BIN="$HOME/.local/bin"
  if [ "$REMOVE" -eq 1 ]; then
    if [ -f "$USER_BIN/mmhmem" ]; then
      if [ "$DRY_RUN" -eq 1 ]; then
        info "[dry-run] rm $USER_BIN/mmhmem"
      else
        rm -f "$USER_BIN/mmhmem"
        info "removed binary copy: $USER_BIN/mmhmem"
      fi
    fi
  elif [ "$BIN" != "$USER_BIN/mmhmem" ]; then
    if [ "$DRY_RUN" -eq 1 ]; then
      info "[dry-run] cp $BIN -> $USER_BIN/mmhmem"
    else
      mkdir -p "$USER_BIN"
      cp "$BIN" "$USER_BIN/mmhmem"
    fi
  fi
  BIN="$USER_BIN/mmhmem"
  if [ "$REMOVE" -eq 0 ]; then
    case ":$PATH:" in
      *":$USER_BIN:"*) ;;
      *) info "note: $USER_BIN is not on your PATH (harness configs use the absolute path, so this only matters for running mmhmem yourself)" ;;
    esac
  fi
fi

# --- summary ----------------------------------------------------------------------
[ -n "$registered" ] || die "nothing to do: no harness selected/detected"
MODE="project: $PROJECT_DIR"
[ "$GLOBAL" -eq 1 ] && MODE="global (all projects)"
echo
if [ "$REMOVE" -eq 1 ]; then
  info "mmhmem local mode removed ($MODE)"
  info "harnesses: ${registered% }"
  info "per-user store left intact (it is your data): ~/.local/share/mmhmem or ~/Library/Application Support/mmhmem — delete it manually if you really want"
else
  info "mmhmem local mode installed ($MODE)"
  info "harnesses: ${registered% }"
  info "binary: $BIN"
  info "store: per-user default (~/.local/share/mmhmem or ~/Library/Application Support/mmhmem); created on first use"
  info "check your savings any time: $BIN stats"
fi
if [ "$DRY_RUN" -eq 1 ]; then
  info "dry run — nothing was written; re-run without --dry-run to apply"
fi
