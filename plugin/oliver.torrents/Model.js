.pragma library

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
  n = Math.max(0, Number(n) || 0)
  var u = ["B", "KiB", "MiB", "GiB", "TiB"]
  var i = 0
  while (n >= 1024 && i < u.length - 1) { n /= 1024; i++ }
  return (i === 0 ? n.toFixed(0) : n.toFixed(1)) + " " + u[i]
}

function rate(n) { return n === undefined || n === null ? "—" : bytes(n) + "/s" }

function eta(secs) {
  secs = Number(secs)
  if (!(secs >= 0) || secs >= 8640000) return "∞"
  var h = Math.floor(secs / 3600), m = Math.floor((secs % 3600) / 60), s = Math.floor(secs % 60)
  if (h > 0) return h + "h" + (m < 10 ? "0" : "") + m + "m"
  if (m > 0) return m + "m" + (s < 10 ? "0" : "") + s + "s"
  return s + "s"
}

function isActive(t) {
  return ["metadata", "checking", "stalled", "downloading", "metaDL", "forcedDL", "stalledDL", "checkingDL", "allocating", "forcedMetaDL"].indexOf(t.state) >= 0
}
function isStopped(t) {
  return ["paused", "done", "stoppedDL", "stoppedUP", "pausedDL", "pausedUP"].indexOf(t.state) >= 0
}
function isDone(t) { return t.state === "done" || t.state === "seeding" || (t.progress_known !== false && Number(t.progress) >= 1) }

function stateLabel(t) {
  switch (t.state) {
    case "downloading": case "forcedDL": return "Downloading"
    case "metadata": case "metaDL": case "forcedMetaDL": return "Fetching metadata"
    case "stalled": case "stalledDL": return "Stalled"
    case "queued": case "queuedDL": return "Queued"
    case "waiting_vpn": case "waitingVPN": return "Waiting for VPN"
    case "checking": case "checkingDL": case "checkingUP": case "checkingResumeData": return "Checking"
    case "seeding": case "uploading": case "forcedUP": return "Seeding"
    case "stalledUP": return "Seeding · idle"
    case "queuedUP": return "Queued · seed"
    case "paused": case "stoppedDL": case "pausedDL": return "Stopped"
    case "done": case "stoppedUP": case "pausedUP": return "Done"
    case "error": return "Error"
    case "missingFiles": return "Missing files"
    default: return t.state || "?"
  }
}

function glyph(t) {
  if (isStopped(t)) return isDone(t) ? "󰄬" : "󰏤"
  if (isActive(t)) return "󰇚"
  if (t.state === "error" || t.state === "missingFiles") return "󰀦"
  return "󰕒"
}

// One-line meta under the torrent name.
function meta(t) {
  if (t.intent !== undefined) {
    var values = []
    if (t.progress !== undefined && t.progress !== null) values.push((t.progress * 100).toFixed(1) + "%")
    if (t.dl_bps !== undefined && t.dl_bps !== null) values.push(rate(t.dl_bps))
    if (t.eta_s !== undefined && t.eta_s !== null) values.push(eta(t.eta_s))
    if (!values.length && t.size !== undefined && t.size !== null) values.push(bytes(t.size))
    values.push(t.seed ? "seeding allowed" : "seed off")
    return values.join(" · ")
  }
  var pct = (Number(t.progress) * 100).toFixed(pctDigits(t))
  var parts = []
  if (t.progress_known === false) return stateLabel(t) + " · progress checked after connection"
  if (t.state === "queuedDL") {
    parts.push("Queued")
    parts.push(pct + "%")
    parts.push(bytes(t.size))
  } else if (isActive(t)) {
    parts.push(pct + "%")
    parts.push(rate(t.dlspeed))
    parts.push(eta(t.eta))
  } else if (isStopped(t)) {
    parts.push(pct + "%")
    parts.push(bytes(t.size))
  } else {
    parts.push(bytes(t.size))
    if (Number(t.upspeed) > 0) parts.push("↑ " + rate(t.upspeed))
    parts.push("ratio " + Number(t.ratio || 0).toFixed(2))
  }
  var flags = (t.seq_dl ? "S" : "") + (t.f_l_piece_prio ? "F" : "")
  if (flags !== "" && !isDone(t)) parts.push(flags)
  parts.push(seedAllowed(t) ? "seeding allowed" : "seed off")
  return parts.join(" · ")
}

function seedAllowed(t) {
  if (typeof t.seed === "boolean") return t.seed
  var tags = String(t.tags || "").split(",").map(function(s) { return s.trim() })
  return tags.indexOf("seed") >= 0 || Number(t.up_limit) !== 32768
}

function pctDigits(t) { return Number(t.progress) < 0.1 ? 2 : 1 }

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
    dl:summary.dl_bps === undefined ? store.transfer.dl_bps : summary.dl_bps,
    up:summary.ul_bps === undefined ? store.transfer.ul_bps : summary.ul_bps}
  if (store.info.state === "parked" || (summary.network && summary.network.state !== "ready")) result.connection = "vpn_waiting"
  return result
}
