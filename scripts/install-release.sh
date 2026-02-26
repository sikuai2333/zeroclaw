#!/usr/bin/env bash
set -euo pipefail

REPO="${ZEROCLAW_UPDATE_REPO:-sikuai2333/zeroclaw}"

if ! command -v zeroclaw >/dev/null 2>&1; then
  echo "error: zeroclaw not found in PATH." >&2
  echo "hint: install zeroclaw first, then run this script for in-app update flow." >&2
  exit 1
fi

# Delegate update logic to zeroclaw built-in updater:
# - release detection
# - sha256 verification
# - atomic replace
# - rollback on restart failure
args=()
for arg in "$@"; do
  case "$arg" in
    --no-onboard)
      # Legacy installer option; no longer needed in built-in updater.
      ;;
    *)
      args+=("$arg")
      ;;
  esac
done

exec zeroclaw update --repo "$REPO" "${args[@]}"
