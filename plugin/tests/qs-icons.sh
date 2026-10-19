#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
export FBT_FONT_FAMILY="${FBT_FONT_FAMILY:-$(fc-match monospace -f '%{family[0]}')}"
export FBT_CAPTURE_DIR="${FBT_CAPTURE_DIR:-$PWD/target/test-logs/visual/$(date +%F)}"
mkdir -p "$FBT_CAPTURE_DIR"
exec python3 plugin/tests/run-qs.py icons plugin/tests/icons
