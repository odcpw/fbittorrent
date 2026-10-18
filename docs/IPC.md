# IPC implementation notes

## Changelog

### P6-07 — live per-torrent upload limits

The typed engine's existing `SetUploadLimit` operation now applies a validated
cap synchronously after its durable ledger save. It retains the torrent handle,
checked data and peers, and returns `EngineResult::Acknowledged` only after the
live limiter has been updated. Legacy `torrents/setUploadLimit` still replies
`Ok.`. Existing CLI `seed` / `noseed` use this path and no longer rebuild a
managed torrent to change its cap. Nonzero caps below 16384 bytes/s or above
the native `u32` maximum are rejected. Zero remains legacy unlimited.

For the planned v1 adapter, `torrents.set_upload_limit {hashes, bps?}` can return
`{state: "done"}` synchronously; keep the result type `enum(done|pending)` for
wire compatibility. `torrents.set_seed` must use the same live setter for
existing handles. The v1 adapter is not present in this checkout, so this entry
does not advertise a new operation, capability or active NDJSON endpoint.
