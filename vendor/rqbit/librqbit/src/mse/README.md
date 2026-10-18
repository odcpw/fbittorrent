# Peer encryption

The maintained native fork uses interoperable BitTorrent Message Stream
Encryption (MSE/PE) on its TCP peer transports. Both the BEP9 magnet metadata
connection and the later payload connection use the configured policy.

| Policy | Outgoing | Incoming |
|---|---|---|
| `off` | Ordinary plaintext | Ordinary plaintext only |
| `prefer` (default) | RC4 first; retry plaintext on a fresh socket if the handshake fails | RC4 or ordinary plaintext |
| `require` | RC4 only; no plaintext fallback | RC4 only |

MSE payload negotiation is RC4-only. Header-only encryption followed by
plaintext payload is rejected. Every attempt has a connect deadline and a
single deadline covering MSE and BEP3. A `prefer` connection makes at most two
attempts. Cancellation is checked before the retry. A failed or interrupted
handshake owns and drops its socket.

Incoming encrypted connections resolve `SHA1("req2" || infohash)` against all
currently managed torrent hashes. The inner BEP3 infohash must equal the
resolved stream key. Pads are bounded at 512 bytes, IA at the wire's 65535-byte
limit, total handshake input at 70 KiB, and each input read at 1024 bytes.
Encryption is continuous across partial writes and vectored reads.

MSE obfuscates BitTorrent traffic. It does not authenticate a peer, provide a
MAC, hide addresses, or encrypt tracker/DHT traffic. VPN socket binding is a
separate session setting.

## Implementation provenance

`protocol/` adapts the pure, socket-free `urtorrent-mse` 0.14.2 sources, licensed
under Apache-2.0. Its original `LICENSE`, copyright headers and `NOTICE` are
retained. The upstream notices credit libtorrent-rasterbar's BSD-3-Clause
method selection/padding logic. Local changes replace the unrelated
metainfo/profile types, allow bounded IA beyond the initial 68 bytes, reject
invalid DH public keys and non-exact crypto selection values, and make cipher
snapshots available for cancellation-correct partial writes. The cloneable
RC4 state is adapted from RustCrypto rc4 0.1.0 under its MIT license; the
license is retained in `protocol/RUSTCRYPTO-LICENSE-MIT`. Tests compare its
bytes with the unmodified RustCrypto crate.

Primary references consulted:

- [urtorrent-mse 0.14.2 source](https://docs.rs/crate/urtorrent-mse/0.14.2/source/)
- [libtorrent key exchange and RC4 implementation](https://github.com/arvidn/libtorrent/blob/RC_2_0/src/pe_crypto.cpp)
- [libtorrent peer handshake](https://github.com/arvidn/libtorrent/blob/RC_2_0/src/bt_peer_connection.cpp)
- [Tixati peer encryption wire description](https://tixati.com/specs/bittorrent/peer_connections/crypto)

The RC4 discard is **1024 bytes**, matching libtorrent's implementation; the
Tixati prose calls this bits, which is inconsistent with the interoperable
implementation.

## Verification

Ordinary tests exercise the vendored state machine and native transport
boundaries:

```sh
cargo test -p librqbit mse
```

The independent oracle test requires an existing Python `libtorrent` module.
It opens only loopback listeners, disables discovery and trackers, generates
its own disposable 192 KiB payload and torrents, verifies the downloaded
bytes, checks libtorrent's RC4 flags, then stops sessions and fixture processes.
It covers both directions, all three policies, magnet metadata, plaintext
fallback, and incoming routing with two managed hashes.

```sh
cargo test -p librqbit libtorrent_metadata_and_payload_interoperability -- --ignored --nocapture
```

Neither fixture uses user torrent data or starts a persistent service.
