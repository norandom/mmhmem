#!/usr/bin/env bash
# Offline checks of the root install.sh stub (reliable-installers task 9.2) under every
# POSIX shell found (sh, bash, dash, busybox sh): it prints the README's one-liner,
# ignores old options, calls no download tool, writes nothing and exits 2, both run as
# a file and piped ("| sh", "| bash -s -- <options>").
# Usage: test-install-stub.sh [STUB [SCRATCH]]
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
stub="${1:-$root/install.sh}"
scratch="${2:-}"
if [ -z "$scratch" ]; then
  mkdir -p "$root/tmp"; scratch="$(mktemp -d "$root/tmp/install-stub.XXXXXX")"
  trap 'rm -rf "$scratch"' EXIT
fi
one_liner='curl -fsSL https://github.com/norandom/mmhmem/releases/latest/download/install.sh | sh'
# README.md may have CRLF endings in a Windows working tree (copied into local Dagger runs).
tr -d '\015' < "$root/README.md" | grep -qxF "$one_liner" || { echo 'README.md lacks the one-liner the stub prints' >&2; exit 1; }
# No network or file-writing command apart from the printed one-liner itself.
if grep -vF "$one_liner" "$stub" | grep -v '^#' | grep -E '(^|[^A-Za-z-])(curl|wget|fetch|nc|python3?|tee|mkdir|cp|mv|rm|touch|chmod|ln)([^A-Za-z-]|$)|>[^&]' > /dev/null; then
  echo 'install.sh stub runs a download or write command' >&2; exit 1
fi
bin="$scratch/bin"; home="$scratch/home"; work="$scratch/work"
mkdir -p "$bin" "$home" "$work"
for tool in curl wget fetch python3 nc; do
  printf '#!/bin/sh\necho "%s $*" >> "%s/calls"\nexit 0\n' "$tool" "$scratch" > "$bin/$tool"
  chmod +x "$bin/$tool"
done
cp "$stub" "$work/install.sh"
before="$(cd "$scratch" && find home work -exec ls -ld {} + 2>/dev/null | sort)"
shells=""
for candidate in sh bash dash; do
  if command -v "$candidate" > /dev/null 2>&1; then shells="$shells $(command -v "$candidate")"; fi
done
if command -v busybox > /dev/null 2>&1; then shells="$shells busybox-sh"; fi
check() { # check WHAT CODE OUTPUT_FILE
  [ "$2" -eq 2 ] || { echo "$1: exit code $2, expected 2" >&2; exit 1; }
  grep -qxF "  $one_liner" "$3" || { echo "$1: one-liner missing: $(cat "$3")" >&2; exit 1; }
  for part in 'has moved' 'did not download or change anything' 'latest signed release' 'mmhmem setup' 'mmhmem update'; do
    grep -qF "$part" "$3" || { echo "$1: '$part' missing" >&2; exit 1; }
  done
}
count=0
for shell in $shells; do
  if [ "$shell" = busybox-sh ]; then run=(busybox sh); else run=("$shell"); fi
  code=0
  (cd "$work" && env HOME="$home" PATH="$bin:$PATH" "${run[@]}" ./install.sh --remove --agent x --hooks claude -Hooks codex --dry-run) > "$scratch/out" 2>&1 || code=$?
  check "$shell file" "$code" "$scratch/out"
  code=0
  (cd "$work" && cat ./install.sh | env HOME="$home" PATH="$bin:$PATH" "${run[@]}") > "$scratch/out" 2>&1 || code=$?
  check "$shell piped" "$code" "$scratch/out"
  code=0
  (cd "$work" && cat ./install.sh | env HOME="$home" PATH="$bin:$PATH" "${run[@]}" -s -- --agent codex --worker-model 'a b') > "$scratch/out" 2>&1 || code=$?
  check "$shell piped -s --" "$code" "$scratch/out"
  count=$((count + 1))
done
[ "$count" -gt 0 ] || { echo 'no POSIX shell found' >&2; exit 1; }
[ ! -e "$scratch/calls" ] || { echo "the stub called: $(cat "$scratch/calls")" >&2; exit 1; }
after="$(cd "$scratch" && find home work -exec ls -ld {} + 2>/dev/null | sort)"
[ "$before" = "$after" ] || { echo 'the stub changed files' >&2; exit 1; }
echo "PASS: install.sh stub prints the release one-liner, ignores options, exits 2, no downloads or writes ($count shell(s))"
