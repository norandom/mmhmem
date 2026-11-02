#!/usr/bin/env bash
# Canonical Windows CI transport only; application execution stays in containers.
set -euo pipefail
source_dir="$1"; call="$2"; export_path="$3"; shift 3
rsync -a --delete --exclude target --exclude tmp --exclude .openidea --exclude .git \
  --exclude data --exclude log --exclude workspace --exclude .env --exclude .kiro \
  --exclude .opencode "$source_dir/" "$HOME/mmhmem-ci/"
cd "$HOME/mmhmem-ci/ci"
if [ "$call" = static-release ] && [ "$export_path" != - ]; then
  exec "$HOME/.local/bin/dagger" call static-release --src .. "$@" export --path "$export_path"
fi
"$HOME/.local/bin/dagger" call "$call" --src .. "$@"
if [ "$export_path" != - ]; then
  "$HOME/.local/bin/dagger" call static-release --src .. "$@" export --path "$export_path"
fi
