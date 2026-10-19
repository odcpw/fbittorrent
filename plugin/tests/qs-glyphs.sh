#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
export FBT_FONT_FAMILY="${FBT_FONT_FAMILY:-$(fc-match monospace -f '%{family[0]}')}"
exec python3 plugin/tests/run-qs.py glyphs plugin/tests/glyphs
