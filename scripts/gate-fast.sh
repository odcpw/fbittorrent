#!/usr/bin/env bash
# Everyday gate: debug tests and one small set of real user workflows.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.rch/shims:$HOME/.cargo/bin:$PATH"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"
exec python3 -B scripts/gate.py fast "$@"
