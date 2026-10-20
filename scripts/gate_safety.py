"""A retry can diagnose a safety failure, but cannot erase it."""
from fnmatch import fnmatchcase

# Explicit families keep newly split cases strict. Timing-only workflows retain
# the normal FLAKY verdict; these cover payload permission, deletion, privacy and
# the single daemon stop rule. Keep this list beside the gate, not UI shapes.
STRICT_CHECKS = (
    'daemon/zero_upload*', 'daemon/*delete*', 'daemon/*remove*', 'daemon/*redact*',
    'daemon/mutation_magnets_and_file_removal', 'daemon/api_torrent_operations',
    'daemon/api_missing_completed_payload', 'daemon/live_missing*', 'daemon/dormant_operations',
    'daemon/bulk_remove_mixed_sizes', 'daemon/remove_requires_flag',
    'daemon/offline_delete_keep_files_ledger', 'daemon/ref_replay_remove',
    'daemon/tracker_url_redacted', 'daemon/tracker_credentials*', 'tracker_privacy',
    'daemon/clipboard_never_logged', 'daemon/torznab*redact*',
    'native/q_*', 'native/quit_restart_down', 'native/native_cli_fields_and_deletion_safety',
    'native/first_q_notice_once', 'native/stop_during_hashing',
    'daemon/down_*', 'daemon/foreground_down', 'daemon/*shutdown*', 'daemon/*stop*',
    'daemon/cleanup_*stop*', 'daemon/*idle_exit*', 'daemon/killmode_mixed',
    'plugin_stop_rule', 'plugin_glance_policy', 'plugin_default_idle', 'plugin_connection_recovery',
    'safety_controls', 'upload_probe_controls',
)
STRICT_RUST_TESTS = (
    'engine::tests::*upload*', 'engine::tests::*delete*',
    'engine::dormant_delete::tests::*', 'engine::tests::tracker_edit::*',
    'engine::tests::*native_lifecycle_hashes_existing_data_stops_restores_and_deletes_durably',
    'engine::tests::*redact*', 'engine::tests::*shutdown*',
    'engine::tests::*stops_with_app*', 'engine::tests::*stop_during_hashing*',
    'engine::tests::*stop_can_persist_intent*',
    'native_runtime::shutdown_tests::*', 'daemon::lifecycle::*', 'daemon::policy::*',
    'api::trackers::*', 'search::torznab::tests::*credentials*',
    'tui::attach::tests::detach_never_requests_owner_shutdown',
    'cli::status::tests::*shutdown*', 'cli::status::tests::*stopping*',
    'api::client::session::tests::*file_deletion*',
    'tui::interaction_tests::destructive_delete_files_requires_y',
    'tui::everyday_tests::commands::*removal*',
)


def strict_check(name):
    return any(fnmatchcase(name, pattern) for pattern in STRICT_CHECKS)


def strict_rust_test(name):
    return any(fnmatchcase(name, pattern) for pattern in STRICT_RUST_TESTS)
