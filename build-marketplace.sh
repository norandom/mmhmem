#!/usr/bin/env bash
set -euo pipefail
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"
case "${1:-}" in
  --manifests-only) lib_generate_plugin_packages ;;
  --release) [ "$#" -eq 2 ] || exit 2; lib_update_marketplace_release "$2" ;;
  --ref) [ "$#" -eq 2 ] || exit 2; lib_generate_marketplace "$2" ;;
  '') lib_generate_marketplace main ;;
  *) echo 'Usage: bash build-marketplace.sh [--ref REF | --release TAG | --manifests-only]' >&2; exit 2 ;;
esac
