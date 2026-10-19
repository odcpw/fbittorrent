# Local tracker diagnostic patch

S09 extends `librqbit-tracker-comms::stats::{redact_url,sanitize}` to remove URL
usernames, passwords and fragments as well as query strings. Known credential
values are removed if an untrusted tracker echoes them outside a URL. Existing
status storage, force coalescing and callers retain their behaviour. librqbit
reexports both helpers so API/CLI/TUI presentation uses the same sanitizer.

Tests: `cargo test --locked -p librqbit-tracker-comms` (including
`userinfo_and_fragments_never_reach_tracker_diagnostics`), fbittorrent's
`api::trackers` tests, and the private tracker journeys for both UIs. The stored
tracker URL remains complete internally; rendered URLs and
diagnostic text contain no userinfo, query string or fragment.


# Tracker path credential privacy (P6-01/P7-04)

`librqbit-tracker-comms::redaction` centralizes the existing public helpers and
redacts hex/base64 path segments of at least 16 characters, named credential path
parameters, percent-encoded forms and credential values echoed in remote failure
prose. Userinfo, queries and fragments remain private. Monitor spans, HTTP/UDP
errors, observations and Debug output all use this producer-side boundary before
logging; original announce URLs and tracker selection remain unchanged internally.

Test: `cargo test --locked -p librqbit-tracker-comms`;
`cargo test --locked -p librqbit --no-default-features --features rust-tls tracker_stats -- --test-threads=1`
exercises real HTTP/UDP failures, unmodified announce credentials and captured traces;
the application's `api::trackers` and engine alias-removal tests cover wire identities; `plugin/tests/tracker_privacy.py`
checks real API/TUI/dashboard/popup and diagnostic output with synthetic secrets.
