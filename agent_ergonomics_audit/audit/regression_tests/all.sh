#!/usr/bin/env bash
# Run every fbittorrent test layer. Exit non-zero if any layer fails.
set -u -o pipefail
cd "$(dirname "$0")/../../.." || exit 1
export PATH="$HOME/.rustup/toolchains/1.98.0-x86_64-unknown-linux-gnu/bin:$PATH"
rc=0
echo "## cargo test (client against a mock WebUI, release-name parser)"; cargo test --release 2>&1 | grep -E 'test result|FAILED|panicked' || rc=1
echo "## CLI agent contract";  agent_ergonomics_audit/audit/regression_tests/run.sh        | tail -1 || rc=1
echo "## TUI smoke (tmux)";     agent_ergonomics_audit/audit/regression_tests/tui_smoke.sh  | tail -1 || rc=1
echo "## widget keys (wtype)";  agent_ergonomics_audit/audit/regression_tests/widget_keys.sh | tail -1 || rc=1
exit $rc
