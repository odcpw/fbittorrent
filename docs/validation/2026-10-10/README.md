# Queue, peer encryption, VPN binding and terminal interaction

Validation date: 2026-10-10. This extends the native engine and Shows/RSS
validation from [2026-10-09](../2026-10-09/README.md).

## Implemented behavior

- Three unfinished downloads may run at once by default. Extra downloads wait
  in saved order; completion, manual pause or removal admits the next download.
  Manual pause remains separate from queue suspension and survives reopening.
  Completed seeders do not consume download slots.
- Peer connections use standard MSE. Prefer attempts encryption, with a fresh
  plaintext connection when negotiation fails. Require rejects plaintext and
  cannot downgrade. Off uses plaintext. Magnet metadata uses the same policy.
- Torrent networking can require ProtonVPN's `proton0` interface or an exact
  interface name. Linux binds engine sockets to that device. Missing, down or
  replaced devices stop the session; the app retains saved history and waits.
  Reconnection creates a new session and restores saved queue/pause intent.
- F6 changes download count, encryption and VPN selection. Queue changes apply
  live. Encryption and binding changes drain and reopen the engine.
- RSS tab hit regions include the brackets drawn around the labels. A click
  selects a row; double-click or Enter activates it. Hover previews a title
  while preserving the selected keyboard target.
- RSS and Shows use `m`/Space for one selected child's read state and Shift+M
  for the selected feed/show. Read state and download state have separate
  symbols. A two-row footer keeps action hints visible alongside feedback,
  with mnemonic labels such as `(O)pen` and explicit shifted shortcuts.
- The engine and all its tasks remain owned by the open app. The normal
  dependency graph contains no Tokio, reqwest or hyper.

## Evidence

| Check | Result and receipt |
| --- | --- |
| App tests | 76 passed, including real local magnet/payload transfer, FIFO admission, completion, pause, deletion, reopening, settings persistence, missing VPN, tab hit regions, hover selection, read undo and the 80-column footer. [app-tests.txt](app-tests.txt) |
| Engine/library tests | 132 passed across librqbit, core, DHT, dual-stack sockets, tracker communications and the runtime; 3 explicitly ignored fixtures. Namespace-only test bodies need the separate runner below. [network-library-tests.txt](network-library-tests.txt) |
| Peer encryption | 16 focused tests passed. The explicitly invoked independent libtorrent oracle passed eight incoming/outgoing metadata and payload cases, checking exact 196,721-byte payloads and actual peer encryption flags. [encryption-tests.txt](encryption-tests.txt) |
| Bound networking | Disposable user/network namespaces exercised IPv4/IPv6, wrong-device refusal, device loss/replacement, accepted sockets, split/vectored I/O, UDP, DNS truncation retry, HTTP redirects and cancellation. Four runtime fixtures and one dual-stack fixture passed; the throughput fixture was invoked separately. [network-tests.txt](network-tests.txt), [http-stack-fix.txt](http-stack-fix.txt) |
| Whole app VPN lifecycle | A private namespace exercised absent/down/up/down/recover/delete/same-name replacement/rapid replacement and app close. Saved records, manual pause and queue intent survived; old session sockets drained. [vpn-app-integration.txt](vpn-app-integration.txt) |
| Real CLI and terminal | A private tmux session with local HTTP RSS/torrent fixtures exercises actual terminal mouse sequences, read/unread scope, refresh, settings, queue changes, VPN waiting, close and reopen. [native-integration.txt](native-integration.txt) |
| Dependency graph | [normal-dependencies.txt](normal-dependencies.txt) records the normal dependencies. Test tools may have different dependencies. |
| UX research | [tui-ux-review.md](../../tui-ux-review.md) records the local design-book evidence, primary client documentation, implemented corrections and remaining recommendations. |

Test counts overlap: the focused encryption tests are also present in the
library suite. These rows are receipts for different checks, not an additive
test total. Ordinary network test runs can return early from namespace-only
fixtures; the separate runner verifies the namespace identity and executes
their bodies.

## Optimized binary and resources

Built with Rust 1.98, `release-perf`, thin LTO, one codegen unit and frame
pointers. The original artifact retains line-table symbols; the installed copy
is stripped and is 17,556,080 bytes. Its SHA256 is
`fc0a1d2b5612ef09eedcf456d667bdab257feca53c3e900c908a2e5205744810`.
[release-build.txt](release-build.txt) and
[release-artifact.json](release-artifact.json) record the build and artifact.
The final CLI/TUI and whole-app VPN checks above ran against this exact stripped
binary before installation.

Ten launch/close runs used fresh temporary state and a 120×30 terminal. The last
run in each scenario measured a ten-second idle window after settling launch
discovery. CLI timing includes process startup. Startup here means the private
engine IPC socket exists, rather than the first visible frame or a ready swarm.

| Measurement | Previous installed binary | New binary, ordinary discovery | New binary, required VPN absent |
| --- | ---: | ---: | ---: |
| Startup median | 8.29 ms | 8.68 ms | 4.59 ms |
| Status CLI median | 2.62 ms | 2.06 ms | 2.11 ms |
| Shutdown median | 2,017 ms | 1,417 ms | 1,467 ms |
| Shutdown maximum | 2,018 ms | 1,969 ms | 1,568 ms |
| Idle RSS | 34.81 MiB | 34.62 MiB | 29.64 MiB |
| Idle proportional memory | 31.52 MiB | 31.33 MiB | 26.33 MiB |
| Launch peak RSS | 82.65 MiB | 81.71 MiB | 77.04 MiB |
| Idle CPU, percent of one core | 0.80% | 1.20% | 0.40% |
| Threads | 8 | 8 | 5 |

These small, same-host samples show the observed resource bounds. They do not
establish a speedup, a statistically significant CPU change or torrent
throughput. Other host work was active, and the final CLI/TUI fixture overlapped
part of the normal launch series. Waiting for a required VPN deliberately omits
the torrent session. P95/P99 estimates are weak with ten launches; the raw
values and process wait channels are retained in
[resources-before.json](resources-before.json),
[resources-after.json](resources-after.json) and
[resources-vpn-waiting.json](resources-vpn-waiting.json).

Four separate 32 MiB native TCP transfers compared bound/unbound sockets with
16 KiB buffers, checking every payload byte. The observed debug-build CPU times
were 78–82 ms in both modes. That bounded result did not justify a readiness
cache; it does not establish WAN throughput. See
[network-tests.txt](network-tests.txt).

## Installation

Installed at `/home/oliver/.local/bin/fbittorrent`, with the exact artifact hash
above, mode 0755. Saved settings are three simultaneous downloads, Prefer
encryption and required ProtonVPN. F6 can change these choices. With ProtonVPN
absent, opening the app displays its waiting state and preserves saved history.

The previous executable and configuration were preserved in
`/home/oliver/.local/share/fbittorrent/backups/2026-10-10-queue-encryption-vpn-20261010T083342Z`.
The app was closed before and after installation. The legacy
`qbittorrent-nox.service` remains inactive and disabled; no app or legacy torrent
process remained after checks. [installation.json](installation.json) records
the binary identity, selected settings and closed status.

The final host check also removed a disconnected IPC socket left from an
earlier app run. Cleanup held the stable app instance lock, checked the socket's
ownership and confirmed connection refusal before unlinking it. The endpoint
is absent and closed status remains unchanged. The guard and final state are
recorded in [final-verification.json](final-verification.json).

The Omarchy plugin folder validated, `rescanPlugins` completed and the shell
answered `ping`. No new screenshot of an open torrent widget was taken because
the user's app remains closed. [widget-validation.json](widget-validation.json)
records that verification limit.

The checkout retains its existing modified and untracked work.
[source-identity.json](source-identity.json) records the branch/base commit and
186 source-file hashes; the base commit alone does not identify these sources.

## Stack defect found and repaired

The wider tracker tests exposed a stack overflow on an ordinary 2 MiB worker
stack. The HTTP facade carried a 28,416-byte future into nested poll frames.
Boxing the bound HTTP/TLS branch at the existing timeout reduced that facade
future to 392 bytes. Ordinary and bound HTTP now run on the actual default
worker stack. A size regression check and the full library run passed without
a production stack-size increase. The initial failure is retained in
[network-library-tests-before-fix.txt](network-library-tests-before-fix.txt);
diagnosis and green reruns are in [http-stack-fix.txt](http-stack-fix.txt).

## Reproduction

Use the matching Rust 1.98 toolchain. All app fixtures use temporary XDG state;
the network runner creates its own user/network namespace.

```sh
cargo test -j2 --locked --bin fbittorrent
cargo test -j2 --locked -p librqbit -p librqbit-core -p librqbit-dht \
  -p librqbit-dualstack-sockets -p librqbit-tracker-comms -p rqbit-runtime \
  --features librqbit/rust-tls --lib

# Find the current test executable names in target/debug/deps.
python3 vendor/rqbit/rqbit-runtime/tests/run-isolated-network-tests.py \
  <rqbit_runtime-test-executable> <dualstack-test-executable>
<librqbit-test-executable> libtorrent_metadata_and_payload_interoperability \
  --ignored --nocapture

RUSTFLAGS='-C force-frame-pointers=yes' cargo build -j2 --locked --profile release-perf
python3 agent_ergonomics_audit/audit/regression_tests/native.py <binary>
python3 agent_ergonomics_audit/audit/regression_tests/vpn.py <binary>
python3 agent_ergonomics_audit/audit/regression_tests/resources.py <binary> \
  --runs 10 --output /tmp/fbittorrent-normal.json
python3 agent_ergonomics_audit/audit/regression_tests/resources.py <binary> \
  --runs 10 --vpn fbmissing0 --output /tmp/fbittorrent-waiting.json
```

## Evidence limits

The torrent transfers use generated local data. They establish the tested
protocol paths and cancellation behavior, including interoperability with the
installed libtorrent build. They do not establish Internet swarm throughput,
every other peer client's behavior or public tracker interoperability.

The binding checks use real kernel sockets in disposable namespaces. The
user's actual ProtonVPN connection was not connected or changed for these
checks. App readiness polls every two seconds; bound I/O rejects device loss
before the app's next visible transition. DNS remains bound and never falls
back to the ordinary system resolver.

RSS, show discovery and search websites use the ordinary app connection.
Engine peers, discovery, trackers, torrent URL fetches and their DNS use the
selected interface. Python provider download hooks are refused while binding
is required because those hooks cannot guarantee device binding; magnets and
direct torrent URLs use the bound engine path.

The UX research's shared action definitions, clickable footer commands,
consistent page navigation and narrower-terminal focused-pane layout remain
recommendations. Current interaction verification covers the implemented
behavior at 80 and 120 columns.
