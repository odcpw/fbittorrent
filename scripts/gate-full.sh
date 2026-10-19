#!/usr/bin/env bash
# Nightly/milestone gate. Keep release compilation under the repository lock.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.rch/shims:$HOME/.cargo/bin:$PATH"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"
exec python3 -B scripts/gate.py full "$@"
