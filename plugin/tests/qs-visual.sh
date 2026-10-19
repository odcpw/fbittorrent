#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
export FBT_FONT_FAMILY="${FBT_FONT_FAMILY:-$(fc-match monospace -f '%{family[0]}')}"
if [[ "${1:-}" == --milestone ]]; then
  shift
  for theme in owner catppuccin-latte; do
    for state in empty downloading paused table-200 ready may-stall; do
      python3 plugin/tests/visual.py popup "$state" --light "$theme" --compare omarchy.network "$@"
    done
  done
  exit 0
fi
exec python3 plugin/tests/visual.py "$@"
