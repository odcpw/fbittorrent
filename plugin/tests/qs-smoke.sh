#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
if [[ "${1:-}" == --palette ]]; then
  shift
  exec python3 plugin/tests/quick.py "$@"
fi
exec python3 plugin/tests/smoke.py "$@"
