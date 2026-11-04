#!/usr/bin/env bash
# Installer opt-in for the read-routing hook (Linux/macOS). HOME is a scratch directory and
# CLAUDE_CONFIG_DIR is unset, so the real ~/.claude/settings.json is never touched.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
mkdir -p "$root/tmp"
scratch="$(mktemp -d "$root/tmp/hooks-engine.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT
mkdir -p "$scratch/home/.claude" "$scratch/bin"
printf '#!/usr/bin/env bash\nprintf "called\\n" >> "$TEST_CLAUDE_LOG"\n' > "$scratch/bin/claude"
printf 'fixture\n' > "$scratch/bin/mmhmem"
chmod +x "$scratch/bin/"*
export TEST_CLAUDE_LOG="$scratch/calls"
settings="$scratch/home/.claude/settings.json"
helper="$scratch/home/.local/bin/mmhmem-manage-hooks.py"
printf '{\n  "model": "opus",\n  "hooks": {\n    "PreToolUse": [\n      {\n        "matcher": "Bash",\n        "hooks": [ { "type": "command", "command": "other-plugin pre" } ]\n      }\n    ]\n  },\n  "ratio": 1.50\n}\n' > "$settings"
cp "$settings" "$scratch/original"
fail() { echo "hooks onboarding: $*" >&2; exit 1; }
run_engine() {
  env -u CLAUDE_CONFIG_DIR -u CLAUDE_CODE_SIMPLE -u CLAUDE_CODE_DISABLE_PROJECT_INSTRUCTIONS \
    XDG_CONFIG_HOME="$scratch/home/.config" HOME="$scratch/home" CODEX_HOME="$scratch/home/.codex" PATH="$scratch/bin:$PATH" \
    bash "$root/deploy/install_local.sh" --global --agent "${TEST_AGENT:-claude}" --binary "$scratch/bin/mmhmem" "$@" < /dev/null 2>&1
}
same_as() { cmp -s "$settings" "$1" || fail "settings changed: $2"; }
entries() { grep -o 'local/bin/mmhmem"' "$settings" | wc -l | tr -d ' '; }

# Default install adds no hook and says how to opt in.
out="$(run_engine)" || fail "default install failed: $out"
same_as "$scratch/original" 'default install'
[ ! -e "$helper" ] || fail 'default install copied the hook helper'
printf '%s' "$out" | grep -q -- '--hooks claude' || fail "default install did not mention the opt-in: $out"

# Dry runs write nothing but still show the entry.
out="$(run_engine --hooks claude --dry-run)" || fail "opt-in dry run failed: $out"
same_as "$scratch/original" 'dry run'
[ ! -e "$helper" ] || fail 'dry run copied the helper'
printf '%s' "$out" | grep -q 'emove it with' || fail "dry run did not show the removal command: $out"
printf '%s' "$out" | grep -q 'What it sees and decides' || fail "dry run did not show what the hook sees: $out"
printf '%s' "$out" | grep -q "$helper" || fail "dry run does not name the persistent helper: $out"
printf '%s' "$out" | grep -q -- '--remove-hooks' || fail "dry run does not name --remove-hooks: $out"
! printf '%s' "$out" | grep -q "$root/deploy/manage_hooks.py" || fail "dry run names the source helper path: $out"

# Opt-in adds exactly one entry; repeating keeps one.
out="$(run_engine --hooks claude)" || fail "opt-in failed: $out"
[ "$(entries)" = 1 ] || fail 'opt-in did not add exactly one entry'
[ -f "$helper" ] || fail 'persistent helper missing'
printf '%s' "$out" | grep -q '/hooks' || fail "missing approval text: $out"
printf '%s' "$out" | grep -q "$helper" || fail 'removal command does not name the persistent helper'
out="$(run_engine --hooks claude)" || fail "repeat opt-in failed: $out"
[ "$(entries)" = 1 ] || fail 'repeat opt-in duplicated the entry'
cp "$settings" "$scratch/installed"

# Dry-run removals and a default re-install leave the entry.
run_engine --remove-hooks --dry-run > /dev/null; same_as "$scratch/installed" '--remove-hooks --dry-run'
run_engine --remove --dry-run > /dev/null; same_as "$scratch/installed" '--remove --dry-run'
run_engine > /dev/null; same_as "$scratch/installed" 'default re-install'

# Hook-only removal restores the settings bytes and leaves the rest of the install.
out="$(run_engine --remove-hooks)" || fail "--remove-hooks failed: $out"
same_as "$scratch/original" '--remove-hooks'
[ ! -e "$helper" ] || fail '--remove-hooks left the persistent helper'
[ -f "$scratch/home/.local/bin/mmhmem" ] || fail '--remove-hooks removed the binary'
[ -f "$scratch/home/.claude/rules/mmhmem.md" ] || fail '--remove-hooks removed the routing rule'

# Full removal removes the entry as well.
run_engine --hooks claude > /dev/null
[ "$(entries)" = 1 ] || fail 'second opt-in missing'
out="$(run_engine --remove)" || fail "full removal failed: $out"
same_as "$scratch/original" 'full removal'

# Codex opt-in is separate, defaults off, and never approves the hook.
printf '#!/usr/bin/env bash\nexit 0\n' > "$scratch/bin/codex"
chmod +x "$scratch/bin/codex"
TEST_AGENT=codex
hooks_file="$scratch/home/.codex/hooks.json"
out="$(run_engine)" || fail "Codex default install: $out"
[ ! -e "$hooks_file" ] || fail 'Codex default added hook'
out="$(run_engine --hooks codex --dry-run)" || fail "Codex dry run: $out"
[ ! -e "$hooks_file" ] || fail 'Codex dry run wrote hook'
printf '%s' "$out" | grep -q -- '--harness codex' || fail 'Codex removal command has wrong harness'
out="$(run_engine --hooks codex)" || fail "Codex install: $out"
printf '%s' "$out" | grep -q '/hooks' || fail 'Codex approval step missing'
cp "$hooks_file" "$scratch/codex-installed"
run_engine --hooks codex > /dev/null
cmp -s "$hooks_file" "$scratch/codex-installed" || fail 'Codex repeat changed entry'
run_engine --remove-hooks > /dev/null
if grep -q 'hook codex' "$hooks_file"; then fail 'Codex removal left entry'; fi
TEST_AGENT=claude

printf '#!/usr/bin/env bash\nexit 0\n' > "$scratch/bin/opencode"
chmod +x "$scratch/bin/opencode"
TEST_AGENT=opencode
plugin="$scratch/home/.config/opencode/plugins/mmhmem-read-routing.js"
out="$(run_engine --hooks opencode --dry-run)" || fail "OpenCode dry-run: $out"
[ ! -e "$plugin" ] || fail 'OpenCode dry run wrote plugin'
out="$(run_engine --hooks opencode)" || fail "OpenCode install: $out"
[ -f "$plugin" ] || fail 'OpenCode plugin absent'
cp "$plugin" "$scratch/plugin-installed"
run_engine --hooks opencode > /dev/null
cmp -s "$plugin" "$scratch/plugin-installed" || fail 'OpenCode repeat changed plugin'
run_engine --remove-hooks > /dev/null
[ ! -e "$plugin" ] || fail 'OpenCode removal left plugin'
TEST_AGENT=claude

# Refusals happen before any write.
if out="$(run_engine --remove --remove-hooks)"; then fail '--remove --remove-hooks accepted'; fi
printf '%s' "$out" | grep -q 'already removes' || fail "--remove --remove-hooks gave no clear message: $out"
for bad in '--hooks codex' '--hooks bogus' '--hooks claude --remove-hooks'; do
  # shellcheck disable=SC2086
  if run_engine $bad > /dev/null; then fail "accepted: $bad"; fi
  same_as "$scratch/original" "refused $bad"
done
if env -u CLAUDE_CONFIG_DIR HOME="$scratch/home" PATH="$scratch/bin:$PATH" \
    bash "$root/deploy/install_local.sh" --project "$scratch" --agent claude --binary "$scratch/bin/mmhmem" --hooks claude < /dev/null > /dev/null 2>&1; then
  fail 'project-scope opt-in accepted'
fi
same_as "$scratch/original" 'project-scope opt-in'
rm -f "$TEST_CLAUDE_LOG"
printf '{\n  "disableAllHooks": true,\n  "model": "opus"\n}\n' > "$settings"
cp "$settings" "$scratch/blocked"
if out="$(run_engine --hooks claude)"; then fail 'policy refusal missing'; fi
printf '%s' "$out" | grep -q 'disableAllHooks' || fail "policy refusal did not name the setting: $out"
printf '%s' "$out" | grep -q 'advisory guidance only' || fail "policy refusal did not say advisory only: $out"
same_as "$scratch/blocked" 'policy refusal'
[ ! -e "$TEST_CLAUDE_LOG" ] || fail 'policy refusal happened after harness registration'
echo 'PASS: hook opt-in default off, dry run, one entry, repeat, hook-only and full removal, refusals before writes'
