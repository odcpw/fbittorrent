#!/usr/bin/env bash
# Run native unit and isolated app integration checks. Legacy/Desktop checks
# require explicit opt-in because they control the configured live service/UI.
set -euo pipefail
cd "$(dirname "$0")/../../.." || exit 1
export PATH="$HOME/.rch/shims:$HOME/.cargo/bin:$PATH"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"
test_log=$(mktemp)
finish() {
  local status=$?
  trap - EXIT
  rm -f "$test_log"
  python3 scripts/testlog-summary.py || status=1
  exit "$status"
}
trap finish EXIT
echo "## Test logging contract and QML marker converter"
python3 agent_ergonomics_audit/audit/regression_tests/fbtlog_test.py
python3 plugin/tests/qs-log.py --self-test
echo "## Repository hygiene"
scripts/check-gitignore.sh
scripts/check-nova3-licences.sh
echo "## Unit, persistence, mouse/render and real peer transfer checks"
# rch gives Cargo a worktree-scoped TMPDIR; Unix sockets need a shorter path.
# The runner changes only test execution, retaining normal remote compilation.
test_host=$(rustc -vV | sed -n 's/^host: //p')
cargo test --locked -p fbittorrent -p rqbit-runtime \
  --config "target.$test_host.runner=[\"env\",\"TMPDIR=/tmp\"]" \
  -- --nocapture 2>&1 | tee "$test_log"
# Native worker failures may not propagate through libtest's calling thread.
# A green exit code must also include a clean background-runtime log.
if rg -n 'thread .*panicked|obligation.*leak|closing native (torrent session|runtime):|native torrent session did not drain' "$test_log"; then
  echo "Native worker failure invalidates the test result" >&2
  exit 1
fi
echo "## Release build"
# The optimized build normally exceeds rch's five-minute default.
RCH_BUILD_TIMEOUT_SEC="${RCH_BUILD_TIMEOUT_SEC:-1200}" cargo build --release --locked
echo "## Isolated native CLI/TUI and RSS checks"
python3 agent_ergonomics_audit/audit/regression_tests/native.py target/release/fbittorrent
if [[ "${FBT_TEST_LEGACY:-0}" == 1 ]]; then
  echo "## Configured legacy CLI and TUI"
  agent_ergonomics_audit/audit/regression_tests/run.sh
  agent_ergonomics_audit/audit/regression_tests/tui_smoke.sh
fi
if [[ "${FBT_TEST_WIDGET:-0}" == 1 ]]; then
  echo "## Live widget keyboard controls"
  agent_ergonomics_audit/audit/regression_tests/widget_keys.sh
fi
