#!/usr/bin/env bash
# P0-11 guard; P2-04 can extend this with the remaining plugin lint rules.
set -euo pipefail
model="$(dirname "$0")/../oliver.torrents/Model.js"
if rg -n 'num_seeds\s*\+\s*["\x27]s["\x27]' "$model"; then
  echo 'Unknown native seed counts must not become an s-suffixed widget count.' >&2
  exit 1
fi
node - "$model" <<'JS'
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const model = {};
vm.runInNewContext(fs.readFileSync(process.argv[2], 'utf8').replace(/^\.pragma library\s*/, ''), model);
const torrent = {state: 'downloading', progress: 0.42, size: 1024 * 1024,
  dlspeed: 2048, upspeed: 0, eta: 65, num_seeds: 0, num_leechs: 7,
  ratio: 0, up_limit: 32768, tags: '', seq_dl: false, f_l_piece_prio: false};
assert.equal(model.meta(torrent), '42.0% · 2.0 KiB/s · 1m05s · seed off');
// Removing only a literal 0 would leave other fabricated seed counts intact.
assert.equal(model.meta({...torrent, num_seeds: 91}), model.meta(torrent));
assert.equal(model.meta({...torrent, progress_known: false, state: 'waitingVPN'}),
  'Waiting for VPN · progress checked after connection');
assert.equal(model.meta({...torrent, state: 'queuedDL'}), 'Queued · 42.0% · 1.0 MiB · seed off');
console.log('Widget metadata omits seed counts and retains rates, ETA, queue and VPN information.');
JS
