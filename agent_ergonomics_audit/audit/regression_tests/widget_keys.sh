#!/usr/bin/env bash
# Real API daemon and popup inside private XDG, network and compositor boundaries.
set -euo pipefail
[[ "${FBT_TEST_WIDGET:-0}" == 1 ]] || { echo 'widget tests skipped (set FBT_TEST_WIDGET=1)'; exit 0; }
cd "$(dirname "$0")/../../.."
exec python3 plugin/tests/popup.py --binary "${FBT_BIN:-$PWD/target/debug/fbittorrent}"
