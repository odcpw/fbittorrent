#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
exec python3 plugin/tests/run-qs.py store plugin/tests/store
