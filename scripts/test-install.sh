#!/usr/bin/env bash
# Offline platform simulation; optional verified candidate runs inside CI only.
set -euo pipefail
bash "$(dirname "$0")/test-codex-registration.sh"
bash "$(dirname "$0")/test-routing-onboarding.sh"
bash "$(dirname "$0")/test-hooks-onboarding.sh"
root="$(cd "$(dirname "$0")/.." && pwd)"
candidate=""; expected_sha256=""; report=""
if [ "$#" -gt 0 ]; then
  [ "$#" -eq 6 ] && [ "$1" = --candidate ] && [ "$3" = --expected-sha256 ] && [ "$5" = --report ] && [ -n "$2" ] && [ -n "$4" ] && [ -n "$6" ] || {
    echo 'usage: test-install.sh [--candidate PATH --expected-sha256 HEX --report PATH]' >&2; exit 2;
  }
  candidate="$2"; expected_sha256="$4"; report="$6"
fi
mkdir -p "$root/tmp"
scratch="$(mktemp -d "$root/tmp/install-shell-tests.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT
mkdir -p "$scratch/bin" "$scratch/home" "$scratch/temp"
for script in install.sh scripts/fixtures/release-install.sh deploy/install_local.sh lib.sh build-marketplace.sh; do bash -n "$root/$script"; done
cat > "$scratch/bin/uname" <<'EOF'
#!/usr/bin/env bash
case "$1" in -s) echo "${TEST_OS:-Darwin}" ;; -m) echo "${TEST_ARCH:-arm64}" ;; esac
EOF
# The root install.sh is a transition delegator: it resolves the latest release tag and
# runs that tag's own installer (scripts/fixtures/release-install.sh stands in for it),
# rewritten to the tag. The curl stub below is the only "network"; the production script
# has no URL override. TEST_TAG, FAIL_API and TEST_INSTALL_SCRIPT steer the stub (tests only).
cat > "$scratch/bin/curl" <<'EOF'
#!/usr/bin/env bash
set -eu
out=""; url=""
while [ "$#" -gt 0 ]; do
  case "$1" in -o) shift; out="$1" ;; -*) ;; *) url="$1" ;; esac
  shift
done
printf '%s\n' "$url" >> "$TEST_LOG"
tag="${TEST_TAG:-v9.9.9}"
emit() { if [ -n "$out" ]; then cp "$1" "$out"; else cat "$1"; fi; }
case "$url" in
  https://api.github.com/repos/norandom/mmhmem/releases/latest)
    [ "${FAIL_API:-0}" = 0 ] || exit 22
    printf '{\n  "url": "x",\n  "tag_name": "%s",\n  "name": "release"\n}\n' "$tag" ;;
  */releases/latest/*|*/HEAD|*/HEAD/*) echo "unpinned URL: $url" >&2; exit 24 ;;
  */releases/download/"$tag"/*)
    [ "${FAIL_BINARY:-0}" = 0 ] || exit 22
    printf 'test binary: never execute\n' > "$out" ;;
  */mmhmem/"$tag"/install.sh)
    emit "${TEST_INSTALL_SCRIPT:-$TEST_ROOT/scripts/fixtures/release-install.sh}" ;;
  */mmhmem/"$tag"/deploy/install_local.sh)
    if [ "${TEST_REAL_ENGINE:-0}" = 1 ]; then
      cp "$TEST_ROOT/deploy/install_local.sh" "$out"
    else
      cat > "$out" <<'ENGINE'
#!/usr/bin/env bash
set -eu
printf '%s\n' "$@" > "$TEST_ARGS"
source_file=""; binary=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --binary) shift; binary="$1" ;;
    --install-source) shift; source_file="$1" ;;
  esac
  shift
done
if [ -n "$source_file" ]; then
  mkdir -p "$(dirname "$binary")"
  cp "$source_file" "$binary"
fi
ENGINE
    fi ;;
  */mmhmem/"$tag"/*) emit "$TEST_ROOT/${url#*/mmhmem/$tag/}" ;;
  *) exit 23 ;;
esac
EOF
chmod +x "$scratch/bin/"*
export TEST_ROOT="$root" TEST_LOG="$scratch/downloads" TEST_ARGS="$scratch/args"
run_bootstrap() {
  env HOME="$scratch/home" TMPDIR="$scratch/temp" PATH="$scratch/bin:$PATH" \
    bash "$root/install.sh" "$@"
}
: > "$TEST_LOG"
run_bootstrap --dry-run --agent codex
# Delegator: tag resolution, pinning, refusals, argument pass-through, nothing written on failure.
head -n 1 "$TEST_LOG" | grep -qx 'https://api.github.com/repos/norandom/mmhmem/releases/latest'
sed -n 2p "$TEST_LOG" | grep -qx 'https://raw.githubusercontent.com/norandom/mmhmem/v9.9.9/install.sh'
refuse() { # refuse WHAT PATTERN [env assignments...]
  local what="$1" pattern="$2"; shift 2
  : > "$TEST_LOG"; rm -f "$TEST_ARGS"
  if env "$@" HOME="$scratch/home" TMPDIR="$scratch/temp" PATH="$scratch/bin:$PATH" bash "$root/install.sh" --dry-run --agent codex 2> "$scratch/refusal"; then
    echo "$what accepted" >&2; exit 1
  fi
  grep -q "$pattern" "$scratch/refusal" || { echo "$what: unexpected message: $(cat "$scratch/refusal")" >&2; exit 1; }
  grep -q 'Nothing was changed' "$scratch/refusal"
  ! grep -q '/deploy/' "$TEST_LOG"
  [ ! -e "$TEST_ARGS" ] && [ ! -e "$scratch/home/.local/bin/mmhmem" ]
}
refuse 'failed tag lookup' 'could not look up' FAIL_API=1
for bad in v1.2 v1.2.3-rc1 latest x1.2.3; do refuse "tag $bad" 'not a stable version' TEST_TAG=$bad; done
{ cat "$root/scripts/fixtures/release-install.sh"; echo "x=https://example.test/mmhmem/HEAD/y"; } > "$scratch/leftover-head.sh"
refuse 'leftover HEAD' 'moving HEAD' TEST_INSTALL_SCRIPT="$scratch/leftover-head.sh"
{ cat "$root/scripts/fixtures/release-install.sh"; echo "x=https://github.com/norandom/mmhmem/releases/latest/other"; } > "$scratch/leftover-latest.sh"
refuse 'leftover releases/latest' 'latest release' TEST_INSTALL_SCRIPT="$scratch/leftover-latest.sh"
{ echo '# mmhmem-transition-delegator'; cat "$root/scripts/fixtures/release-install.sh"; } > "$scratch/recursive.sh"
refuse 'recursive delegator' 'recurse' TEST_INSTALL_SCRIPT="$scratch/recursive.sh"
# Arguments and the rewritten text reach the tag's installer verbatim.
cat > "$scratch/probe.sh" <<'PROBE'
printf '%s\n' "$@" > "$TEST_ARGS"
echo "raw=https://raw.githubusercontent.com/norandom/mmhmem/HEAD rel=https://github.com/norandom/mmhmem/releases/latest/download" > "$TEST_ARGS.text"
PROBE
: > "$TEST_LOG"
env TEST_INSTALL_SCRIPT="$scratch/probe.sh" HOME="$scratch/home" PATH="$scratch/bin:$PATH" bash "$root/install.sh" --agent codex --worker-model 'a b' --dry-run
printf '%s\n' --agent codex --worker-model 'a b' --dry-run | cmp - "$TEST_ARGS"
grep -qx 'raw=https://raw.githubusercontent.com/norandom/mmhmem/v9.9.9 rel=https://github.com/norandom/mmhmem/releases/download/v9.9.9' "$TEST_ARGS.text"
rm -f "$TEST_ARGS" "$TEST_ARGS.text"
: > "$TEST_LOG"
run_bootstrap --dry-run --agent codex
! grep -q releases/latest/ "$TEST_LOG"
! grep -q '/HEAD' "$TEST_LOG"
grep -q 'mmhmem/v9.9.9/install.sh' "$TEST_LOG"
grep -q 'deploy/opencode/mmhmem-read-routing.js' "$TEST_LOG"
[ ! -e "$scratch/home/.local/bin/mmhmem" ]
grep -q -- --global "$TEST_ARGS"
run_bootstrap --agent codex
grep -q mmhmem-macos-aarch64 "$TEST_LOG"
grep -q 'test binary' "$scratch/home/.local/bin/mmhmem"
export FAIL_BINARY=1
if run_bootstrap --agent codex; then echo 'Failed download accepted' >&2; exit 1; fi
grep -q 'test binary' "$scratch/home/.local/bin/mmhmem"
unset FAIL_BINARY
export TEST_ARCH=x86_64
run_bootstrap --agent codex
grep -q mmhmem-macos-x86_64 "$TEST_LOG"
# Simulate ordinary Linux even if the CI container's host kernel is Microsoft.
real_grep="$(command -v grep)"
export TEST_REAL_GREP="$real_grep"
cat > "$scratch/bin/grep" <<'EOF'
#!/usr/bin/env bash
if [ "$*" = '-qi microsoft /proc/version' ]; then
  [ "${TEST_WSL:-0}" = 1 ]; exit $?
fi
exec "$TEST_REAL_GREP" "$@"
EOF
chmod +x "$scratch/bin/grep"
export TEST_OS=Linux
run_bootstrap --agent codex
grep -q mmhmem-linux-x86_64 "$TEST_LOG"
export TEST_WSL=1
if run_bootstrap --agent codex; then echo 'WSL install accepted' >&2; exit 1; fi
unset TEST_WSL
export TEST_ARCH=aarch64
if run_bootstrap --agent codex; then echo 'Unsupported Linux architecture accepted' >&2; exit 1; fi
export TEST_ARCH=x86_64
: > "$TEST_LOG"
run_bootstrap --remove --agent codex
[ "$(wc -l < "$TEST_LOG")" -eq 6 ]
grep -q manage_hooks.py "$TEST_LOG"
grep -q manage_routing.py "$TEST_LOG"
grep -q routing_onboarding.py "$TEST_LOG"
[ -z "$(ls -A "$scratch/temp")" ]
bash "$root/install.sh" --help >/dev/null
# Exercise the real registration engine without executing a binary or editing user state.
printf '#!/usr/bin/env bash\nexit 0\n' > "$scratch/bin/codex"
chmod +x "$scratch/bin/codex"
env HOME="$scratch/home" PATH="$scratch/bin:$PATH" bash "$root/deploy/install_local.sh" \
  --global --agent codex --binary "$scratch/missing/mmhmem" --dry-run > "$scratch/engine.log"
grep -q 'config.toml' "$scratch/engine.log"
grep -q '.agents/skills/unified-memory' "$scratch/engine.log"
grep -q '.agents/skills/mmhmem-recap/SKILL.md' "$scratch/engine.log"
env HOME="$scratch/home" PATH="$scratch/bin:$PATH" bash "$root/deploy/install_local.sh" \
  --global --agent codex --remove --dry-run > "$scratch/engine.log"
grep -q 'local mode removed' "$scratch/engine.log"
[ ! -d "$scratch/home/.agents" ]
echo 'PASS: Bash syntax, dry run, global default, Linux/Apple assets, platform refusals, failed-download preservation, removal, cleanup'
PATH="$scratch/bin:$PATH" python3 "$root/scripts/test-install-menus.py" "$root" "$scratch"
python3 "$root/scripts/test-worker-check.py"
python3 "$root/scripts/test_manage_hooks.py"
python3 "$root/scripts/test_manage_hooks_codex.py"
python3 "$root/scripts/test_manage_hooks_opencode.py"
if [ -n "$candidate" ]; then
  python3 "$root/ci/scripts/probe_install.py" --binary "$candidate" --expected-sha256 "$expected_sha256" --source "$root" --report "$report"
fi
