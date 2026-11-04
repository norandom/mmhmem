#!/usr/bin/env bash
# mmhmem-transition-delegator
#
# TRANSITION INSTALLER. This file replaces the old repository-HEAD installer so
# that the one-liner
#   curl -fsSL https://raw.githubusercontent.com/norandom/mmhmem/HEAD/install.sh | bash
# keeps working while HEAD moves ahead of the published release. It installs the
# latest published release exactly as that release's own installer does, with
# every download pinned to that release's tag:
#   1. resolve the latest stable release tag (vX.Y.Z) from the GitHub API;
#   2. fetch that tag's own install.sh;
#   3. rewrite its /HEAD and releases/latest/download URLs to the tag;
#   4. run it with your arguments (stdin stays the script; menus use /dev/tty).
# It refuses, changing nothing, if the tag lookup or fetch fails, if a /HEAD
# reference would remain, or if the fetched script is itself this delegator.
# Task 9.2 in .kiro/specs/reliable-installers replaces this file with a stub that
# points at the new release assets.
set -eu

die() { printf 'mmhmem: %s\nNothing was changed.\n' "$1" >&2; exit 1; }

fetch() {
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$1"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO- "$1"
  else
    return 127
  fi
}

main() {
  local api tag text
  api=$(fetch 'https://api.github.com/repos/norandom/mmhmem/releases/latest') ||
    die 'could not look up the latest mmhmem release (need curl or wget and network access).'
  tag=$(printf '%s\n' "$api" | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1)
  printf '%s\n' "$tag" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+$' ||
    die "the latest release tag '$tag' is not a stable version (vX.Y.Z)."
  text=$(fetch "https://raw.githubusercontent.com/norandom/mmhmem/$tag/install.sh") ||
    die "could not download the installer for release $tag."
  [ -n "$text" ] || die "the installer for release $tag is empty."
  case "$text" in
    *mmhmem-transition-delegator*) die "release $tag ships the transition delegator itself; refusing to recurse." ;;
  esac
  text=$(printf '%s\n' "$text" | sed \
    -e "s#https://raw\.githubusercontent\.com/norandom/mmhmem/HEAD#https://raw.githubusercontent.com/norandom/mmhmem/$tag#g" \
    -e "s#https://github\.com/norandom/mmhmem/releases/latest/download#https://github.com/norandom/mmhmem/releases/download/$tag#g")
  case "$text" in
    */HEAD*) die "the installer for release $tag still refers to the moving HEAD; refusing." ;;
    *releases/latest*) die "the installer for release $tag still refers to the latest release; refusing." ;;
  esac
  printf '%s\n' "$text" | bash -s -- "$@"
}

# Everything is defined before main runs, so a piped script is read completely.
main "$@"
exit $?
