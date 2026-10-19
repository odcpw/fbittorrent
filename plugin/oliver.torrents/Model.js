.pragma library
.import "Glyphs.js" as Glyphs
.import "Formats.js" as Formats
.import "Palette.js" as Palette

function parseStatus(raw) {
  try {
    var d = JSON.parse(raw)
    if (!d) return { ok: false, error: "bad response" }
    return d
  } catch (e) {
    return { ok: false, error: String(e) }
  }
}

function bytes(n) {
  if (n===undefined || n===null) return "—"
  n = Math.max(0, Number(n) || 0)
  var u = ["B", "KiB", "MiB", "GiB", "TiB"]
  var i = 0
  while (n >= 1024 && i < u.length - 1) { n /= 1024; i++ }
  return (i === 0 ? n.toFixed(0) : n.toFixed(1)) + " " + u[i]
}

function rate(n, compact) { return Formats.rate(n,!!compact) }

function isActive(t) {
  return ["metadata", "checking", "stalled", "downloading"].indexOf(t.state) >= 0
}
function isStopped(t) {
  return ["paused", "done"].indexOf(t.state) >= 0
}
function isDone(t) { return t.state === "done" || t.state === "seeding" }

var words={metadata:"Metadata",checking:"Checking",downloading:"Downloading",stalled:"Stalled",queued:"Queued",paused:"Paused",done:"Done",seeding:"Seeding",waiting_vpn:"Waiting for VPN",error:"Error",disk_full:"Disk full",ready:"Ready to watch",added_before:"Added before",in_list:"In your list",unread:"Unread",pending:"Pending",reason:"Reason"}
var stateRoles={metadata:"metadata",checking:"checking",downloading:"download",stalled:"stalled",queued:"queued",paused:"paused",done:"done",seeding:"upload",waiting_vpn:"vpn",error:"error",disk_full:"error",ready:"ready",added_before:"add",in_list:"add",unread:"unread",pending:"paused",reason:"error"}
function stateLabel(t) {
  if(t.playable && t.playable.ready && isActive(t)){
    var p=t.playable
    return p.stall_free===true || (p.stall_free===undefined && Number(p.buffered_s)>=120) ? "Ready to watch" : "Playable · may stall"
  }
  if (t.state==="error" && t.reason==="disk_full") return "Disk full"
  if (words[t.state]) return words[t.state]
  return t.state || "?"
}

function glyph(t) {
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

// One-line meta under the torrent name.
function meta(t) {
  var values=[]
  if(t.progress!==undefined && t.progress!==null)values.push(percent(t.progress,t.state==="done" || t.state==="seeding"))
  if(t.dl_bps!==undefined && t.dl_bps!==null)values.push(rate(t.dl_bps))
  if(t.eta_s!==undefined && t.eta_s!==null)values.push(Formats.eta(t.eta_s,false))
  if(!values.length && t.size!==undefined && t.size!==null)values.push(Formats.size(t.size,false))
  values.push(t.seed ? "seeding allowed" : "seed off")
  return values.join(" · ")
}

function heroMeta(s) {
  if (!s.ok) return "Backend offline"
  if (s.loaded === false) return "Loading torrents…"
  if (s.connection === "vpn_waiting") return "Waiting for VPN"
  if (s.active > 0 || Number(s.up) > 0 || Number(s.dl) > 0) return "↓ " + rate(s.dl) + "   ↑ " + rate(s.up)
  if (s.total === 0) return "No torrents"
  return "Idle"
}

function detailPill(s) {
  if (!s.ok) return ""
  if (s.active > 0) return s.active + " ACTIVE"
  if (s.seeding > 0) return s.seeding + " SEEDING"
  if (s.queued > 0) return s.queued + " QUEUED"
  return ""
}

function apiStatus(store, online) {
  var summary = store.summary, counts = summary.counts || store.counts
  var result = {ok:online, loaded:!!summary.counts || store.hasTorrents, total:counts.total || 0,
    active:(counts.downloading || 0) + (counts.metadata || 0) + (counts.checking || 0) + (counts.stalled || 0),
    seeding:counts.seeding || 0, stopped:(counts.paused || 0) + (counts.done || 0), queued:counts.queued || 0,
    history:Array.isArray(summary.history) ? summary.history : [],
    dl:summary.dl_bps === undefined ? store.transfer.dl_bps : summary.dl_bps,
    up:summary.ul_bps === undefined ? store.transfer.ul_bps : summary.ul_bps}
  if (store.info.state === "parked" || (summary.network && summary.network.state !== "ready")) result.connection = "vpn_waiting"
  return result
}

function barRate(n, compact) {
  if (n===undefined || n===null) return compact ? "" : "—"
  var value=Math.max(0,Number(n)), units=compact ? ["B","K","M","G","T"] : ["B/s","KB/s","MB/s","GB/s","TB/s"], index=0
  while (value>=1000 && index<units.length-1) {value/=1000;index++}
  return value.toFixed(index>0 && value<10 ? 1 : 0) + (compact ? "" : " ") + units[index]
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
