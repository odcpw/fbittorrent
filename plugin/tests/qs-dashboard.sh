#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
if [[ "${1:-}" == --owner-review ]]; then
  shift
  exec python3 plugin/tests/owner-review.py "$@"
fi
if [[ "${1:-}" == --journeys ]]; then
  shift
  exec python3 plugin/tests/journeys.py "$@"
fi
if [[ "${1:-}" == --pieces ]]; then
  shift
  exec python3 plugin/tests/pieces.py "$@"
fi
if [[ "${1:-}" == --grid ]]; then
  shift
  exec python3 plugin/tests/grid.py "$@"
fi
if [[ "${1:-}" == --preferences ]]; then
  shift
  exec python3 plugin/tests/preferences.py "$@"
fi
if [[ "${1:-}" == --search ]]; then
  shift
  exec python3 plugin/tests/search.py "$@"
fi
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
