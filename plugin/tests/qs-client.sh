#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
# The runner creates a private HOME and all five XDG dirs; it never loads the live shell.
exec python3 plugin/tests/run-qs.py client plugin/tests/client
