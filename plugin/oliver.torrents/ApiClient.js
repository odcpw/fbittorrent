.pragma library

// Transport-independent API 1 session; one instance per shell service.
var IDEMPOTENT = ["torrents.start", "torrents.stop", "torrents.set_seed", "torrents.set_files",
  "torrents.set_upload_limit", "torrents.add", "torrents.commit", "torrents.discard",
  "torrents.force_start", "app.settings.set", "transfer.set_limits", "transfer.set_alt",
  "search.set_provider", "rss.set_read", "rss.mark_feed_read", "shows.set_quality",
  "shows.set_read", "shows.mark_show_read", "torrents.reannounce", "torrents.set_download_limit",
  "torrents.add_tracker", "torrents.remove_tracker"]

function utf8Bytes(s) {
  var bytes = 0
  for (var i = 0; i < s.length; ++i) {
    var c = s.charCodeAt(i)
    if (c < 128) bytes++
    else if (c < 2048) bytes += 2
    else if (c >= 0xd800 && c <= 0xdbff && i + 1 < s.length &&
             s.charCodeAt(i + 1) >= 0xdc00 && s.charCodeAt(i + 1) <= 0xdfff) { bytes += 4; i++ }
    else bytes += 3
  }
  return bytes
}

function create(io, options) {
  var o = options || {}, now = o.now || Date.now
  var subscriptionGeneration=0
  var c = { online: false, instance: "", caps: [], hello: null, pending: Object.create(null), actions: Object.create(null),
    queue: [], awaiting: Object.create(null), subs: Object.create(null), topicRevs: Object.create(null), nextId: 1, nextRef: 1, started: now(),
    lastFrame: now(), helloSent: false, connecting: false, reconnecting: false,
    retryIndex: 0, retryDelay: 500, graceful: false, popupOpen: false,
    dashboardVisible: false, protocolMismatch: false, details: [], mode: "connecting", lastError: null, pollAt: 0 }
  function changed() { if (o.changed) o.changed(c) }
  function deliver(frame, request) { if (o.frame) o.frame(frame, request) }
  function fail(entry, failure, status) {
    if (entry.ref) c.actions[entry.ref] = Object.assign({},c.actions[entry.ref],{op:entry.op,status:status || "failed",error:failure})
    if (entry.callback) entry.callback(null, failure)
    if(!entry.quiet)c.lastError = failure
  }
  function write(entry, resend) {
    var frame = { id: c.nextId++, op: entry.op }
    if (entry.args !== undefined) frame.args = entry.args
    if (entry.ref) frame.ref = entry.ref
    if (resend && entry.instance) frame.instance = entry.instance
    var text = JSON.stringify(frame) + "\n"
    if (utf8Bytes(text) > 65536) {
      fail(entry, { code: "invalid_args", message: "request exceeds 65536 bytes", retryable: false })
      return false
    }
    entry.id = frame.id
    if (!resend) entry.deadline = now() + (entry.op === "torrents.add" ? 100000 : 20000)
    c.pending[frame.id] = entry
    io.write(text)
    io.flush()
    return frame.id
  }
  function pump() {
    while (c.online && c.queue.length && Object.keys(c.pending).length < 8) write(c.queue.shift(), false)
  }
  c.request = function(op, args, settings) {
    settings = settings || {}
    var cap = op === "torrents.files" || op === "torrents.set_files" ? "files" : op.indexOf("torrents.") === 0 ? "torrents" : ""
    if (c.online && cap && c.caps.indexOf(cap) < 0) {
      var unsupported = {code:"unsupported", message:"This daemon lacks " + cap + "; restart to update", retryable:false}
      c.lastError = unsupported
      if (settings.callback) settings.callback(null, unsupported)
      changed(); return false
    }
    var entry = { op: op, args: args, callback: settings.callback, instance: c.instance, quiet:settings.quiet===true && !settings.mutating,
      ref: settings.mutating ? "qs-" + c.started + "-" + c.nextRef++ : undefined }
    if (entry.ref) {
      var refs = Object.keys(c.actions)
      if (refs.length >= 256) {
        var done = refs.filter(function(ref) { return c.actions[ref].status !== "pending" })
        if (!done.length) { if (entry.callback) entry.callback(null, {code:"busy", message:"too many pending actions", retryable:true}); return false }
        delete c.actions[done[0]]
      }
      c.actions[entry.ref] = { op: op, args: args, status: "pending" }
    }
    if (!c.online && op !== "hello") {
      fail(entry, { code: "engine_unavailable", message: "fbittorrent is not running", retryable: true })
      changed(); return false
    }
    if (Object.keys(c.pending).length >= 8) {
      if (c.queue.length >= 64) fail(entry, { code: "busy", message: "client queue is full", retryable: true })
      else c.queue.push(entry)
      changed(); return entry.ref || false
    }
    var id = write(entry, false)
    changed()
    return entry.ref || id
  }
  c.connected = function() {
    if (c.helloSent) return
    c.helloSent = true; c.connecting = true; c.graceful = false; c.lastFrame = now()
    write({ op: "hello", args: { proto: 1, client: { name: "oliver.torrents",
      version: o.version || "0.2.0", kind: "bar" }, interactive: c.dashboardVisible } }, false)
    changed()
  }
  c.disconnected = function() {
    var wasOnline = c.online
    subscriptionGeneration++; c.online = false; c.helloSent = false; c.connecting = false; c.subs = Object.create(null); c.topicRevs = Object.create(null)
    if (!c.graceful && wasOnline) c.reconnecting = true
    c.mode = c.protocolMismatch ? "protocol_mismatch" : c.reconnecting ? "reconnecting" : "stopped"
    // Unsent actions have no unknown outcome and are never replayed.
    while (c.queue.length) fail(c.queue.shift(), { code: "engine_unavailable", message: "connection closed before send", retryable: true })
    Object.keys(c.pending).forEach(function(id) {
      var e = c.pending[id]
      if (!e.ref || IDEMPOTENT.indexOf(e.op) < 0) {
        fail(e, { code: "timeout", message: "no answer", retryable: true }, "no answer")
        delete c.pending[id]
      }
    })
    Object.keys(c.awaiting).forEach(function(ref) {
      var e = c.awaiting[ref]
      if (IDEMPOTENT.indexOf(e.op) < 0) {
        fail(e, { code: "timeout", message: "no answer", retryable: true }, "no answer")
        delete c.awaiting[ref]
      }
    })
    changed()
  }
  c.attempt = function() {
    var ladder = [500, 1000, 2000, 4000, 8000, 10000]
    c.retryDelay = ladder[Math.min(c.retryIndex++, ladder.length - 1)]
    if (c.retryDelay === 10000) c.reconnecting = false
    if (!io.isConnected()) io.connect()
    changed()
    return c.retryDelay
  }
  c.reset = function(message) {
    c.lastError = { code: "invalid_message", message: message, retryable: false }
    io.disconnect(); c.disconnected()
  }
  function presence() {
    var args = { interactive: c.dashboardVisible, glance: c.popupOpen }
    if (c.caps.indexOf("dashboard_presence") >= 0) args.dashboard_visible = c.dashboardVisible
    return args
  }
  c.visibility = function(popup, dashboard, details) {
    c.popupOpen = !!popup; c.dashboardVisible = !!dashboard; c.details = details || []
    if (c.online) {
      c.request("app.client", presence())
      c.subscribeAll()
    }
  }
  function topics() {
    var wanted = ["summary"]
    if (c.popupOpen || c.dashboardVisible) wanted.push("torrents")
    if(c.dashboardVisible)wanted.push("events")
    if(c.dashboardVisible && c.caps.indexOf("settings")>=0)wanted.push("settings")
    if(c.dashboardVisible && c.caps.indexOf("shows")>=0)wanted.push("shows")
    if(c.dashboardVisible && c.caps.indexOf("rss")>=0)wanted.push("rss.feeds")
    return wanted
  }
  c.subscribeAll = function() {
    subscriptionGeneration++
    Object.keys(c.subs).forEach(function(sub) { c.request("unsubscribe", { sub: Number(sub) }) })
    c.subs = Object.create(null); c.topicRevs = Object.create(null)
    if (c.caps.indexOf("subscribe") < 0 || c.mode === "poll") { c.mode = "poll"; c.pollAt = 0; c.poll(); return }
    var generation=++subscriptionGeneration
    var groups=[topics()].concat(c.details.slice(0,12).map(function(topic){return [topic]}))
    groups.forEach(function(group){
      c.request("subscribe", {topics:group,interval_ms:c.popupOpen || c.dashboardVisible ? 1000 : 5000}, {
        callback:function(result,failure){
          if(result && generation!==subscriptionGeneration){c.request("unsubscribe",{sub:result.sub});return}
          if(failure && failure.code==="unknown_op"){c.mode="poll";c.pollAt=0;c.poll()}
          else if(result)c.subs[result.sub]={topics:result.topics,rev:0,resync:false}
          changed()
        }
      })
    })
  }

  c.resubscribe = function(topic) {
    // A subscription shares one revision counter across its topics.
    // Rebuild its entire atomic group, never retain a partially ordered group.
    c.subscribeAll()
    if (o.resubscribe) o.resubscribe(topic)
  }
  c.poll = function() {
    if (!c.online || c.mode !== "poll" || now() < c.pollAt) return
    c.pollAt = now() + (c.popupOpen || c.dashboardVisible ? 1000 : 5000)
    var active = Object.keys(c.pending).map(function(id) { return c.pending[id].op })
    if (active.indexOf("app.info") < 0) c.request("app.info")
    if (c.caps.indexOf("torrents") >= 0 && active.indexOf("torrents.list") < 0) c.request("torrents.list", {})
    if(c.dashboardVisible && c.caps.indexOf("shows")>=0 && active.indexOf("shows.list")<0)c.request("shows.list")
    if(c.dashboardVisible && c.caps.indexOf("rss")>=0 && active.indexOf("rss.feeds")<0)c.request("rss.feeds")
    if (active.indexOf("app.network") < 0) c.request("app.network")
    if (active.indexOf("transfer.info") < 0) c.request("transfer.info")
  }
  c.tick = function() {
    if (c.helloSent && now() - c.lastFrame >= 45000) { c.reset("45 seconds without a frame"); return }
    Object.keys(c.pending).forEach(function(id) {
      var e = c.pending[id]
      if (now() >= e.deadline) { delete c.pending[id]; fail(e, { code: "timeout", message: "no answer", retryable: true }, "no answer") }
    })
    Object.keys(c.awaiting).forEach(function(ref) {
      var e = c.awaiting[ref]
      if (now() >= e.deadline) { delete c.awaiting[ref]; fail(e, { code: "timeout", message: "no answer", retryable: true }, "no answer") }
    })
    pump(); c.poll(); changed()
  }
  c.receive = function(line) {
    if (utf8Bytes(line) + 1 > 1048576) { c.reset("server line exceeds 1048576 bytes"); return }
    var frame
    try { frame = JSON.parse(line) } catch (e) { c.reset("invalid JSON from daemon"); return }
    if (!frame || typeof frame !== "object" || Array.isArray(frame)) { c.reset("expected an API object"); return }
    c.lastFrame = now()
    if (frame.ev) {
      if(frame.ev==="bye" && frame.reason==="shutdown") {
        Object.keys(c.pending).forEach(function(id){
          var entry=c.pending[id]
          if(entry.op==="app.shutdown"){if(entry.ref)c.actions[entry.ref].status="confirmed";delete c.pending[id]}
        })
        Object.keys(c.awaiting).forEach(function(ref){if(c.awaiting[ref].op==="app.shutdown"){c.actions[ref].status="confirmed";delete c.awaiting[ref]}})
      }
      if (frame.ev === "bye") { c.graceful = true; c.reconnecting = false; c.mode = "stopped"; c.online = false }
      if (frame.ev === "error") c.lastError = frame.error
      if (frame.ev === "mutation.result") {
        delete c.awaiting[frame.ref]
        var action = c.actions[frame.ref]
        if (action) { action.status = frame.outcome === "timeout" ? "no answer" : frame.outcome; action.error = frame.error }
      }
      if (frame.sub !== undefined) {
        if (typeof frame.topic !== "string" || !Number.isSafeInteger(frame.sub) || frame.sub < 1 ||
            !Number.isSafeInteger(frame.rev) || frame.rev < 1 ||
            (frame.ev === "snapshot.chunk" && (!Array.isArray(frame.items) || frame.items.length > 256))) {
          c.reset("invalid subscription frame"); return
        }
        var sub = c.subs[frame.sub]
        if (!sub) return
        if (frame.rev !== sub.rev + 1) { c.resubscribe(frame.topic); changed(); return }
        sub.rev = frame.rev; c.topicRevs[frame.topic] = frame.rev
      }
      deliver(frame); changed(); return
    }
    var entry = c.pending[frame.id]
    if (!entry) return
    delete c.pending[frame.id]
    if (typeof frame.ok !== "boolean" || (frame.ok && (!frame.result || typeof frame.result !== "object" || Array.isArray(frame.result))) ||
        (!frame.ok && (!frame.error || typeof frame.error.code !== "string" || typeof frame.error.message !== "string" || typeof frame.error.retryable !== "boolean"))) {
      c.reset("invalid response envelope"); return
    }
    if (frame.ok && frame.result.state === "pending" && (!entry.ref || frame.result.ref !== entry.ref)) {
      c.reset("pending response has missing or mismatched ref"); return
    }
    if (entry.op === "hello" && frame.ok) {
      if (frame.result.proto !== 1) { c.protocolMismatch = true; c.mode = "protocol_mismatch"; io.disconnect(); changed(); return }
      if (!/^[0-9a-f]{32}$/.test(frame.result.instance) || !Array.isArray(frame.result.caps) ||
          frame.result.caps.some(function(cap) { return typeof cap !== "string" })) { c.reset("invalid hello result"); return }
      c.protocolMismatch = false
      var previous = c.instance
      c.instance = frame.result.instance; c.caps = frame.result.caps || []; c.hello = frame.result
      c.online = true; c.connecting = false; c.reconnecting = false; c.retryIndex = 0; c.mode = "subscribe"
      var retry = Object.keys(c.pending).map(function(id) { return c.pending[id] })
        .concat(Object.keys(c.awaiting).map(function(ref) { return c.awaiting[ref] }))
      c.pending = Object.create(null); c.awaiting = Object.create(null)
      retry.forEach(function(e) {
        if (e.ref && IDEMPOTENT.indexOf(e.op) >= 0 && now() < e.deadline) write(e, true)
        else fail(e, { code: "timeout", message: "no answer", retryable: true }, "no answer")
      })
      if (o.instance && previous !== c.instance) o.instance(c.instance)
      c.request("app.client", presence())
      c.subscribeAll()
    } else if (!frame.ok) fail(entry, frame.error)
    else if (entry.ref && c.actions[entry.ref]) {
      c.actions[entry.ref].status = frame.result.state === "pending" ? "pending" : "confirmed"
      if (frame.result.state === "pending") c.awaiting[entry.ref] = entry
    }
    if (entry.callback && frame.ok) entry.callback(frame.result, null)
    deliver(frame, entry); pump(); changed()
  }
  c.state = function() {
    return { online: c.online, instance: c.instance, subs: c.topicRevs,
      pending: Object.keys(c.actions).filter(function(ref) { return c.actions[ref].status === "pending" }), view: null }
  }
  return c
}
