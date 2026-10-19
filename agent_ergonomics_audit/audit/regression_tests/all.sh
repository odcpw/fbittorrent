#!/usr/bin/env bash
# Run native unit and isolated app integration checks. Legacy/Desktop checks
# require explicit opt-in because they control the configured live service/UI.
set -euo pipefail
if [[ "${1:-}" == --help || "${1:-}" == -h ]]; then
  cat <<'HELP'
Usage: all.sh [--help]
Default: logger/marker tests, hygiene, Rust tests, release build, isolated native
CLI/TUI tests, CI/resource/summary regression tests, then the JSONL summary table.
Local release/all.sh runs must use the AGENTS.md release-lock/memory-scope wrapper.

Environment flags (opt-ins default to 0; set exactly 1 to enable):
  FBT_TEST_QS=1         Quickshell store/real-daemon smoke (P2-02/P2-06); requires
                       Quickshell, Qt 6.12 and Omarchy shell modules.
  FBT_TEST_VISUAL=1     plugin/tests/qs-visual.sh captures (P2-08); requires the
                       private headless Sway and the installed Omarchy components.
  FBT_TEST_WIDGET=1     widget_keys.sh; requires the live shell, plugin and wtype.
  FBT_TEST_VPN=1        vpn.py in its isolated user/network namespace; requires
                       unshare, iproute2 and permission to create namespaces.
  FBT_TEST_RESOURCES=1  Build release-perf, measure idle/active CPU and RSS, and
                       compare baselines/resources.json on its reference host.
  FBT_TEST_DAEMON_RESOURCES=1  Build release-perf and compare the daemon 10/100-torrent,
                       0/1/8-subscriber CPU/RSS matrix (three 60 s runs; ~24 min).
  FBT_TEST_VERBOSE=1    Mirror every JSONL line to stderr, including pane polls.
  FBT_TEST_FORCE_FAIL=<suite>/<case>
                       Force that case's first step to fail; retain its logs.
  FBT_TEST_LEGACY=1     Legacy CLI checks; controls the configured live service.

Unavailable opted-in fixture scripts fail explicitly. QS/visual scripts arrive
in their named P2 beads. No live-service or live-widget suite runs by default.
HELP
  exit 0
fi
if [[ $# != 0 ]]; then
  echo "Unknown argument: $1 (use --help)" >&2
  exit 2
fi
cd "$(dirname "$0")/../../.." || exit 1
export PATH="$HOME/.rch/shims:$HOME/.cargo/bin:$PATH"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"
# Reset all scheduled JSONL suites before any gate can fail. Archived bead
# evidence and unrelated suites are neither removed nor included in this run.
summary_args=(--suite fbtlog_test --suite qs_log_test --suite hygiene --suite native --suite daemon --suite tui_smoke --suite tui_smoke_setup --suite tui_smoke_control --suite ci_gates)
if [[ "${FBT_TEST_QS:-0}" == 1 ]]; then summary_args+=(--suite gate_qs_store --suite gate_qs_smoke); fi
if [[ "${FBT_TEST_VISUAL:-0}" == 1 ]]; then summary_args+=(--suite gate_visual); fi
if [[ "${FBT_TEST_WIDGET:-0}" == 1 ]]; then summary_args+=(--suite gate_widget); fi
if [[ "${FBT_TEST_VPN:-0}" == 1 ]]; then summary_args+=(--suite gate_vpn); fi
if [[ "${FBT_TEST_RESOURCES:-0}" == 1 ]]; then summary_args+=(--suite resources); fi
if [[ "${FBT_TEST_DAEMON_RESOURCES:-0}" == 1 ]]; then summary_args+=(--suite daemon_budget); fi
python3 scripts/testlog-summary.py --reset "${summary_args[@]}"
test_log=$(mktemp)
finish() {
  local status=$?
  trap - EXIT
  rm -f "$test_log"
  if ! python3 scripts/testlog-summary.py "${summary_args[@]}"; then
    if [[ "$status" == 0 ]]; then status=1; fi
  fi
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
echo "## Widget peer-label lint and model checks"
plugin/tests/lint.sh
echo "## Isolated native CLI/TUI and RSS checks"
python3 scripts/check-widget-online.py --self-test
python3 agent_ergonomics_audit/audit/regression_tests/native.py target/release/fbittorrent
echo "## Isolated daemon lazy-restore checks"
python3 agent_ergonomics_audit/audit/regression_tests/daemon.py target/release/fbittorrent --no-systemd \
  --case lazy_restore_no_checks --case dormant_seed_checks \
  --case dormant_missing_files --case dormant_open_no_check
echo "## Native terminal smoke at 80/120/160 columns"
python3 agent_ergonomics_audit/audit/regression_tests/tui_smoke.py target/release/fbittorrent
python3 agent_ergonomics_audit/audit/regression_tests/tui_smoke_test.py target/release/fbittorrent
echo "## CI, summary and resource-gate regression checks"
python3 agent_ergonomics_audit/audit/regression_tests/ci_test.py
gate() {
  local suite=$1
  shift
  python3 agent_ergonomics_audit/audit/regression_tests/ci.py --suite "$suite" -- "$@"
}
if [[ "${FBT_TEST_QS:-0}" == 1 ]]; then
  gate gate_qs_store bash plugin/tests/qs-store.sh
  gate gate_qs_smoke bash plugin/tests/qs-smoke.sh
fi
if [[ "${FBT_TEST_VISUAL:-0}" == 1 ]]; then
  gate gate_visual bash plugin/tests/qs-visual.sh
fi
if [[ "${FBT_TEST_VPN:-0}" == 1 ]]; then
  # vpn.py owns its config/data/cache/runtime directories; supply isolated state
  # here as well, without changing that older fixture's implementation.
  python3 agent_ergonomics_audit/audit/regression_tests/ci.py --suite gate_vpn --isolate-state -- \
    python3 agent_ergonomics_audit/audit/regression_tests/vpn.py target/release/fbittorrent
fi
if [[ "${FBT_TEST_RESOURCES:-0}" == 1 || "${FBT_TEST_DAEMON_RESOURCES:-0}" == 1 ]]; then
  RCH_BUILD_TIMEOUT_SEC="${RCH_BUILD_TIMEOUT_SEC:-1200}" cargo build --profile release-perf --locked
fi
if [[ "${FBT_TEST_RESOURCES:-0}" == 1 ]]; then
  python3 agent_ergonomics_audit/audit/regression_tests/resources.py target/release-perf/fbittorrent \
    --baseline agent_ergonomics_audit/audit/regression_tests/baselines/resources.json \
    --output target/test-logs/resources/measurement.json
fi
if [[ "${FBT_TEST_DAEMON_RESOURCES:-0}" == 1 ]]; then
  python3 agent_ergonomics_audit/audit/regression_tests/resources.py target/release-perf/fbittorrent \
    --daemon --matrix --runs 3 --seconds 60 \
    --baseline agent_ergonomics_audit/audit/regression_tests/baselines/resources.json \
    --output target/test-logs/daemon_budget/measurement.json
fi
if [[ "${FBT_TEST_LEGACY:-0}" == 1 ]]; then
  echo "## Configured legacy CLI"
  agent_ergonomics_audit/audit/regression_tests/run.sh
fi
if [[ "${FBT_TEST_WIDGET:-0}" == 1 ]]; then
  echo "## Live widget keyboard controls"
  gate gate_widget bash agent_ergonomics_audit/audit/regression_tests/widget_keys.sh
fi
