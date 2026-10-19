.pragma library
.import "Glyphs.js" as Glyphs
.import "Formats.js" as Formats
.import "Palette.js" as Palette

function rate(n, compact) { return Formats.rate(n,!!compact) }

var ACTIVE_STATES=["metadata","checking","stalled","downloading"]
function isActiveState(state){return ACTIVE_STATES.indexOf(state)>=0}
function activeCount(counts){var total=0;for(var i=0;i<ACTIVE_STATES.length;i++)total+=counts[ACTIVE_STATES[i]] || 0;return total}
function isActive(t) {
  return !t.staged && isActiveState(t.state)
}
function isStopped(t) {
  return !t.staged && ["paused", "done"].indexOf(t.state) >= 0
}
function isDone(t) { return !t.staged && (t.state === "done" || t.state === "seeding") }

var words={metadata:"Metadata",checking:"Checking",downloading:"Downloading",stalled:"Stalled",queued:"Queued",paused:"Paused",done:"Done",seeding:"Uploading",waiting_vpn:"Waiting for VPN",error:"Error",disk_full:"Disk full",ready:"Ready to watch",added_before:"Added before",in_list:"In your list",unread:"Unread",pending:"Pending",reason:"Reason"}
var stateRoles={metadata:"metadata",checking:"checking",downloading:"download",stalled:"stalled",queued:"queued",paused:"paused",done:"done",seeding:"upload",waiting_vpn:"vpn",error:"error",disk_full:"error",ready:"ready",added_before:"add",in_list:"add",unread:"unread",pending:"paused",reason:"error"}
function stateLabel(t) {
  if(t.staged)return "Reviewing"
  if(t.playable && t.playable.ready && isActive(t)){
    var p=t.playable
    return p.stall_free===true || (p.stall_free===undefined && Number(p.buffered_s)>=120) ? "Ready to watch" : "Playable · may stall"
  }
  if (t.state==="error" && t.reason==="missing_files") return "Files missing"
  if (t.state==="error" && t.reason==="disk_full") return "Disk full"
  if (words[t.state]) return words[t.state]
  return t.state || "?"
}

function glyph(t) {
  if(t.staged)return Glyphs.glyph("add")
  if(t.playable && t.playable.ready && isActive(t))return Glyphs.glyph("ready")
  if (stateRoles[t.state]) return Glyphs.glyph(stateRoles[t.state])
  if (isStopped(t)) return isDone(t) ? Glyphs.done : Glyphs.paused
  if (isActive(t)) return Glyphs.download
  if (t.state === "error") return Glyphs.error
  return Glyphs.upload
}

// Counts describe connected peers only. An absent bitfield-derived seed count
// stays unknown even when the connected-peer count is known zero.
function peerCount(t,compact) {
  var peers=t.peers,seeds=t.seeds
  var hasPeers=typeof peers==="number" && isFinite(peers) && peers>=0
  var hasSeeds=typeof seeds==="number" && isFinite(seeds) && seeds>=0
  if(compact)return (hasSeeds ? String(seeds) : "—")+"/"+(hasPeers ? String(peers) : "—")
  if(hasSeeds && hasPeers)return seeds+" seeds / "+peers+" peers connected"
  if(hasPeers)return peers+" peers connected · seeds unknown"
  if(hasSeeds)return seeds+" seeds · connected peers unknown"
  return "Connected peers unknown"
}

function heroMeta(s) {
  if (!s.ok) return "Backend offline"
  if (s.loaded === false) return "Loading torrents…"
  if (s.connection === "vpn_waiting") return "Waiting for VPN"
  if (s.active > 0 || Number(s.up) > 0 || Number(s.dl) > 0) return "↓ " + rate(s.dl) + "   ↑ " + rate(s.up)
  if (s.total === 0) return "No torrents"
  return "Idle"
}

function apiStatus(store, online) {
  var summary = store.summary, counts = summary.counts || store.counts
  var result = {ok:online, loaded:!!summary.counts || store.hasTorrents, total:counts.total || 0,
    active:activeCount(counts),
    seeding:counts.seeding || 0, stopped:(counts.paused || 0) + (counts.done || 0), queued:counts.queued || 0,
    history:Array.isArray(summary.history) ? summary.history : [],
    dl:summary.dl_bps === undefined ? store.transfer.dl_bps : summary.dl_bps,
    up:summary.ul_bps === undefined ? store.transfer.ul_bps : summary.ul_bps}
  if (store.info.state === "parked" || (summary.network && summary.network.state !== "ready")) result.connection = "vpn_waiting"
  return result
}

function plainTip(text) {
  text=String(text).replace(/</g,"‹").replace(/>/g,"›")
  return text.length>512 ? text.slice(0,511)+"…" : text
}
function sourceName(source) {
  var value=source.magnet || source.url || source.path || "Torrent"
  if (source.magnet) {
    var match=/[?&]dn=([^&#]*)/i.exec(value)
    if (match) {try {return decodeURIComponent(match[1].replace(/\+/g," "))} catch(error) {return match[1]}}
    return "Magnet link"
  }
  return value.split(/[?#]/)[0].split("/").pop() || "Torrent"
}

function size(value,compact) {return Formats.size(value,!!compact)}
function eta(value,compact) {return Formats.eta(value,!!compact)}
function percent(value,complete) {return Formats.percent(value,!!complete)}
function count(value) {return Formats.count(value)}
function when(value,now) {return Formats.when(value,now)}
function ago(value,now) {return Formats.ago(value,now)}
function dim(fg,bg) {return Palette.dim(fg,bg)}
function accentText(accent,bg,fg) {return Palette.accentText(accent,bg,fg)}
