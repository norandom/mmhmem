#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
python3 "$root/scripts/test_routing_onboarding.py"
scratch="$(mktemp -d "$root/tmp/routing-engine.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT
mkdir -p "$scratch/home" "$scratch/project" "$scratch/bin"
printf '#!/usr/bin/env bash\necho "2.2.5"\n' > "$scratch/bin/opencode"
printf '#!/usr/bin/env bash\nprintf "called\\n" >> "$TEST_CLAUDE_LOG"\n' > "$scratch/bin/claude"
chmod +x "$scratch/bin/"*
export TEST_CLAUDE_LOG="$scratch/calls"
run_engine() {
 env HOME="$scratch/home" PATH="$scratch/bin:$PATH" bash "$root/deploy/install_local.sh" --project "$scratch/project" --agent claude,opencode --binary /bin/true "$@"
}
run_engine --dry-run > "$scratch/output"
[ ! -e "$scratch/calls" ]
[ ! -e "$scratch/project/AGENTS.md" ]
printf '<!-- mmhmem:routing:start -->\n' > "$scratch/project/AGENTS.md"
if run_engine > "$scratch/output" 2>&1; then echo 'Malformed ownership accepted' >&2; exit 1; fi
[ ! -e "$scratch/calls" ]
rm "$scratch/project/AGENTS.md"
run_engine > "$scratch/output"
grep -q 'mmhmem:routing:start' "$scratch/project/AGENTS.md"
[ -f "$scratch/project/.claude/rules/mmhmem.md" ]
run_engine > "$scratch/output"
[ "$(grep -c 'mmhmem:routing:start' "$scratch/project/AGENTS.md")" -eq 1 ]
run_engine --remove > "$scratch/output"
[ ! -e "$scratch/project/.claude/rules/mmhmem.md" ]
[ ! -s "$scratch/project/AGENTS.md" ]
echo 'PASS: Unix routing scope, selection, repeat, removal, dry-run and engine preflight ordering'
