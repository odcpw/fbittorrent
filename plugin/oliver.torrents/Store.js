.pragma library

// Minimal socket-backed cache. P2-02 adds generic topic reduction and sorting.
function create() {
  var s = { torrents: Object.create(null), summary: {}, info: {}, transfer: {}, snapshots: {} }
  s.apply = function(frame, request) {
    if (request && frame.ok) {
      if (request.op === "hello" || request.op === "app.info") s.info = frame.result
      if (request.op === "transfer.info") s.transfer = frame.result
      if (request.op === "torrents.list") {
        s.torrents = Object.create(null)
        frame.result.torrents.forEach(function(row) { s.torrents[row.hash] = row })
        return true
      }
    }
    if (frame.topic === "summary" && frame.ev === "state") s.summary = frame.data
    if (frame.topic !== "torrents") return false
    if (frame.ev === "snapshot.begin") s.snapshots[frame.sub] = Object.create(null)
    if (frame.ev === "snapshot.chunk") frame.items.forEach(function(row) { s.snapshots[frame.sub][row.hash] = row })
    if (frame.ev === "snapshot.end") { s.torrents = s.snapshots[frame.sub]; delete s.snapshots[frame.sub]; return true }
    if (frame.ev === "delta") {
      ;(frame.added || []).forEach(function(row) { s.torrents[row.hash] = row })
      ;(frame.removed || []).forEach(function(hash) { delete s.torrents[hash] })
      Object.keys(frame.changed || {}).forEach(function(hash) {
        var row = s.torrents[hash]
        if (!row) return
        Object.keys(frame.changed[hash]).forEach(function(key) {
          if (frame.changed[hash][key] === null) delete row[key]
          else row[key] = frame.changed[hash][key]
        })
      })
      return true
    }
    return false
  }
  s.rows = function() { return Object.keys(s.torrents).map(function(hash) { return s.torrents[hash] }) }
  return s
}
