#!/usr/bin/env bash
# CLI qualification against isolated, owned API v1 daemons.
set -euo pipefail
binary=${1:-target/debug/fbittorrent}
exec python3 "$(dirname "$0")/agent_contract.py" "$binary"
