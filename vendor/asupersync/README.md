# fbittorrent native runtime

This is fbittorrent's private fork of Asupersync 0.5.0. It provides the native
structured-concurrency runtime, cancellation, TCP/UDP, TLS and HTTP primitives
used by the app and rqbit. It is not a general-purpose upstream distribution.

The application enables `native-runtime`, `tls`, `tls-native-roots` and
`tls-webpki-roots`. `epoch-diagnostics` adds optional measurement counters.
`lab-runtime` retains the deterministic runtime used by rqbit's cancellation
and resource-release test; it is not part of the application release profile.
Defaults are empty.

Unused ATP/QUIC/HTTP3, database, gRPC/messaging, CLI/doctor, swarm tooling and
OTLP implementations have been removed. Remaining shared implementations retain
their behavior. See [FBITTORRENT_PATCH.md](FBITTORRENT_PATCH.md) for the supported
scope, local runtime fixes, validation commands and upstream test-source limits.

The original [LICENSE](LICENSE), including its OpenAI/Anthropic rider, is
retained unmodified. Provenance and the original crate digest are recorded in
[FBITTORRENT_PATCH.md](FBITTORRENT_PATCH.md).
