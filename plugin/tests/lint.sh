#!/usr/bin/env bash
# Plugin invariants, including the P0-07 and P0-11 regressions.
set -euo pipefail
root="${FBT_PLUGIN_SOURCE:-$(dirname "$0")/../oliver.torrents}"
model="$root/Model.js"
python3 "$(dirname "$0")/token-lint.py" "$root"
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
const torrent = {state:'downloading', progress:0.42, dl_bps:2048, eta_s:65, seed:false};
assert.equal(model.meta(torrent), '42% · 2.0 KB/s · 1 min · seed off');
assert.equal(model.peerCount(torrent,true), '—/—');
assert.equal(model.peerCount({...torrent,peers:7},false), '7 peers connected · seeds unknown');
assert.equal(model.peerCount({...torrent,peers:7,seeds:1},true), '1/7');
assert.equal(model.peerCount({...torrent,peers:0,seeds:0},true), '0/0');
assert.equal(model.meta({state:'waiting_vpn',seed:false}), 'seed off');
assert.equal(model.isDone({state:'downloading',progress:1}),false);
console.log('Native widget formatting retains unknown observations and uses only real connected seed/peer counts.');
JS
