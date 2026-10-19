#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
# During cross-track integration the immutable transcript may already be on main
# while this UI track still has the older server base. Read it without merging
# or editing a contract. qs_env copies it normally once the integration lands.
if [[ ! -f contracts/ipc/v1/search.ndjson ]]; then
  mkdir -p target/test-logs/fbt-p3-12-wkf
  git show main:contracts/ipc/v1/search.ndjson > target/test-logs/fbt-p3-12-wkf/search.from-main.ndjson
  export FBT_TEST_SEARCH_TRANSCRIPT="$PWD/target/test-logs/fbt-p3-12-wkf/search.from-main.ndjson"
fi
exec python3 plugin/tests/run-qs.py search_flow plugin/tests/search-flow
