#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
if [[ "${1:-}" == --details ]]; then
  shift
  exec python3 plugin/tests/details.py "$@"
fi
if [[ "${1:-}" == --destination-picker ]]; then
  shift
  exec python3 plugin/tests/destination.py "$@"
fi
if [[ "${1:-}" == --add-sheet ]]; then
  shift
  exec python3 plugin/tests/add_sheet.py "$@"
fi
exec python3 plugin/tests/dashboard.py "$@"
