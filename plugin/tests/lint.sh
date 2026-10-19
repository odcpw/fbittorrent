#!/usr/bin/env bash
# Plugin invariants, including the P0-07 and P0-11 regressions.
set -euo pipefail
root="${FBT_PLUGIN_SOURCE:-$(dirname "$0")/../oliver.torrents}"
model="$root/Model.js"
python3 "$(dirname "$0")/check-structure.py" "$root"
python3 "$(dirname "$0")/check-frame-handlers.py" "$root"
python3 "$(dirname "$0")/../../scripts/check-widget-online.py" --self-test "$root/Service.qml"
if rg -n 'num_seeds\s*\+\s*["\x27]s["\x27]' "$model"; then
  echo 'Unknown native seed counts must not become an s-suffixed widget count.' >&2
  exit 1
fi
node - "$model" <<'JS'
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const source = p => fs.readFileSync(p, 'utf8').replace(/^\.(?:pragma|import).*$/gm, '');
const model = {};
for (const name of ['Glyphs', 'Formats', 'Palette']) {
  const file = require('node:path').join(require('node:path').dirname(process.argv[2]), name+'.js');
  if (fs.existsSync(file)) {model[name]={}; vm.runInNewContext(source(file),model[name]);}
}
vm.runInNewContext(source(process.argv[2]), model);
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
