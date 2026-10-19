#!/usr/bin/env bash
# Alias for the full local milestone/nightly gate.
set -euo pipefail
exec "$(dirname "$0")/../../../scripts/gate-full.sh" "$@"
