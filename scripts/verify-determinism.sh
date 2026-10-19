#!/usr/bin/env bash
# Disposable daemon; each verb's byte comparison and SHA256 are logged as E.1.
set -euo pipefail
exec python3 "$(dirname "$0")/../agent_ergonomics_audit/audit/regression_tests/agent_contract.py" "${1:-target/debug/fbittorrent}" --case json_determinism
