# Local tracker diagnostic patch

S09 extends `librqbit-tracker-comms::stats::{redact_url,sanitize}` to remove URL
usernames, passwords and fragments as well as query strings. Known credential
values are removed if an untrusted tracker echoes them outside a URL. Existing
status storage, force coalescing and callers retain their behaviour. librqbit
reexports both helpers so API/CLI/TUI presentation uses the same sanitizer.

Tests: `cargo test --locked -p librqbit-tracker-comms` (including
`userinfo_and_fragments_never_reach_tracker_diagnostics`), fbittorrent's
`api::trackers` tests, and the private tracker journeys for both UIs. The stored
tracker URL remains complete for explicit socket operations; rendered URLs and
diagnostic text contain no userinfo, query string or fragment.
