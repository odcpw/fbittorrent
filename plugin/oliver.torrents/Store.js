.pragma library

var STATES = ["metadata", "checking", "downloading", "stalled", "queued", "paused", "done", "seeding", "waiting_vpn", "error"]
function dictionary() { return Object.create(null) }
function equal(a, b) {
  if (a === b) return true
  if (!a || !b || typeof a !== "object" || typeof b !== "object") return false
  var left = Object.keys(a), right = Object.keys(b)
  if (left.length !== right.length) return false
  for (var i = 0; i < left.length; ++i) if (!equal(a[left[i]], b[left[i]])) return false
  return true
}
function safeField(key) { return key !== "__proto__" && key !== "constructor" && key !== "prototype" }
function ready(row) { return !!(row && row.playable && row.playable.ready) }
// Name comparisons dominate cold UI sorts; avoid dynamic field lookup and
// comparing 40-byte hashes unless the case-folded names actually tie.
function compareNames(a,b) {return a.name<b.name ? -1 : a.name>b.name ? 1 : a.hash<b.hash ? -1 : a.hash>b.hash ? 1 : 0}
function compareNamesReverse(a,b) {return a.name<b.name ? 1 : a.name>b.name ? -1 : a.hash<b.hash ? -1 : a.hash>b.hash ? 1 : 0}
function create(options) {
  var o = options || {}, clock = o.now || Date.now
  var s = { torrents: dictionary(), summary: {}, network: {}, settings: {}, info: {}, transfer: {},
    badges: {}, topics: dictionary(), subscriptions: dictionary(), snapshots: dictionary(), calls: dictionary(),
    mutations: dictionary(), events: [], eventSeen: dictionary(), cards: dictionary(), counts: {total:0, ready:0, staged:0},
    changes: [], hasTorrents: false, membershipChanged: false, proto: undefined, compatible: false, instance: "",
    generation: 0, membershipVersion: 0, sortVersions: {added:0,name:0,progress:0,rate:0,size:0,state:0,eta:0,up:0,peers:0,ratio:0}, sortNames: dictionary(), sortValues: dictionary(), keys: [], keysDirty: true, sortCache: dictionary(), lastError: null }
  STATES.forEach(function(state) { s.counts[state] = 0 })
  var errors=dictionary(), unacknowledged=dictionary()
  s.firstError=function() {for (var hash in errors) return s.torrents[hash]; return null}
  s.firstReady=function() {for (var hash in unacknowledged) return s.torrents[hash]; return null}
  function recount(row, sign) {
    if (!row) return
    s.counts.total += sign
    if (s.counts[row.state] !== undefined) s.counts[row.state] += sign
    if (ready(row)) s.counts.ready += sign
    if (row.staged) s.counts.staged += sign
  }
  function touched(hash, fields, membership) {
    s.changes.push({hash:hash, fields:fields})
    s.membershipChanged = s.membershipChanged || membership
    if (membership) s.membershipVersion++
    var sortFields = {added_at:"added", name:"name", progress:"progress", dl_bps:"rate", size:"size", state:"state", eta_s:"eta",ul_bps:"up",peers:"peers",ratio:"ratio"}
    ;(fields || []).forEach(function(field) { if (sortFields[field]) s.sortVersions[sortFields[field]]++ })
    s.generation++
  }
  function upsert(incoming, patch) {
    var hash = incoming.hash
    if (!/^[0-9a-f]{40}$/.test(hash)) throw Error("invalid torrent hash")
    var old = s.torrents[hash], added = !old, fields = [], next = old || {}
    var oldState = old && old.state, oldReady = ready(old), oldStaged = !!(old && old.staged)
    var keys = Object.keys(incoming)
    if (!patch && old) keys = keys.concat(Object.keys(old).filter(function(k) { return incoming[k] === undefined }))
    keys.forEach(function(key) {
      if (!safeField(key)) throw Error("invalid field")
      var value = incoming[key]
      if (patch && value === null) value = undefined
      if (!equal(next[key], value)) {
        fields.push(key)
        if (value === undefined) delete next[key]
        else next[key] = value
      }
    })
    if (!added && !fields.length) return
    if (added) { s.torrents[hash] = next; recount(next, 1); s.keysDirty = true }
    else {
      if (oldState !== next.state) { if (s.counts[oldState] !== undefined) s.counts[oldState]--; if (s.counts[next.state] !== undefined) s.counts[next.state]++ }
      if (oldReady !== ready(next)) s.counts.ready += ready(next) ? 1 : -1
      if (oldStaged !== !!next.staged) s.counts.staged += next.staged ? 1 : -1
    }
    if (added || fields.indexOf("name") >= 0) s.sortNames[hash] = String(next.name || "").toLocaleLowerCase()
    if (next.state==="error") errors[hash]=true; else delete errors[hash]
    if (next.playable && next.playable.ready && next.playable.acked===false) unacknowledged[hash]=true; else delete unacknowledged[hash]
    var rank = STATES.indexOf(next.state)
    var values = s.sortValues[hash] || {hash:hash}
    values.added_at = next.added_at; values.name = s.sortNames[hash]; values.progress = next.progress
    values.dl_bps = next.dl_bps; values.size = next.size; values.state = rank < 0 ? STATES.length : rank; values.eta_s = next.eta_s; values.ul_bps=next.ul_bps;values.peers=next.peers;values.ratio=next.ratio
    s.sortValues[hash] = values
    touched(hash, fields, added)
  }
  function remove(hash) {
    var row = s.torrents[hash]
    if (!row) return
    recount(row, -1); delete errors[hash]; delete unacknowledged[hash]; delete s.torrents[hash]; delete s.sortNames[hash]; delete s.sortValues[hash]; s.keysDirty = true; touched(hash, null, true)
  }
  function replaceTorrents(rows) {
    s.hasTorrents = true
    var present = dictionary()
    rows.forEach(function(row) { present[row.hash] = true; upsert(row, false) })
    Object.keys(s.torrents).forEach(function(hash) { if (!present[hash]) remove(hash) })
  }
  function topicKey(topic, row) {
    if (topic === "torrents") return row.hash
    if (/^search:/.test(topic)) return row.key
    if (/^torrent:.*\.trackers$/.test(topic)) return row.url
    if (/^torrent:.*\.peers$/.test(topic)) return row.addr
    if (/^torrent:.*\.files$/.test(topic)) return String(row.index)
    if (topic === "events" || topic === "log") return String(row.seq)
    return String(row.hash !== undefined ? row.hash : row.id !== undefined ? row.id : row.imdb)
  }
  function trimLog(){
    var target=s.topics.log || dictionary(),keys=Object.keys(target)
    if(keys.length>1000){keys.sort(function(a,b){return Number(a)-Number(b)});keys.slice(0,keys.length-1000).forEach(function(k){delete target[k]})}
  }
  s.releaseTopics=function(topics){topics.forEach(function(topic){if(/^(torrent:|search:)/.test(topic))delete s.topics[topic]})}
  s.logEntries=function(hash){var rows=s.topics.log || {};return Object.keys(rows).map(function(k){return rows[k]}).filter(function(r){return !hash || r.hash===hash}).sort(function(a,b){return a.seq-b.seq})}
  function keepMutation(reference, value) {
    var refs = Object.keys(s.mutations)
    if (!s.mutations[reference] && refs.length >= 256) delete s.mutations[refs[0]]
    s.mutations[reference] = value
  }
  function badge(topic) {
    if(topic!=="shows" && topic!=="rss.feeds")return
    var total=0
    Object.keys(s.topics[topic] || {}).forEach(function(key){total+=Number(s.topics[topic][key].unread || 0)})
    var next=Object.assign({},s.badges);next[topic==="shows" ? "shows" : "rss"]=total;s.badges=next
  }
  function domainList(topic,rows) {
    var keyed=dictionary()
    rows.forEach(function(row){keyed[topicKey(topic,row)]=row})
    s.topics[topic]=keyed;badge(topic)
  }
  function event(frame) {
    if (frame.ev === "mutation.result") keepMutation(frame.ref, {op:frame.op, status:frame.outcome, error:frame.error, result:frame.result})
    if (frame.seq !== undefined && !s.eventSeen[frame.seq]) {
      s.eventSeen[frame.seq] = true; s.events.push(frame)
      if (s.events.length > 1000) delete s.eventSeen[s.events.shift().seq]
    }
  }
  s.noteRequest = function(frame) {
    s.calls[frame.id] = frame
    if (frame.ref) keepMutation(frame.ref, {op:frame.op, status:"pending"})
  }
  s.apply = function(frame, request) {
    s.changes.length = 0; s.membershipChanged = false
    if (frame.ev) {
      if (frame.sub === undefined) { event(frame); return false }
      var sub = s.subscriptions[frame.sub]
      if (!sub) return false
      if (sub.resync) return false
      if (frame.rev !== sub.rev + 1) {
        sub.resync = true
        if (o.resubscribe) o.resubscribe(frame.topic)
        return false
      }
      sub.rev = frame.rev
      var topic = frame.topic, snapshotKey = frame.sub + ":" + topic
      var activeSnapshot = Object.keys(s.snapshots).some(function(key) { return key.indexOf(frame.sub + ":") === 0 })
      if ((frame.ev === "state" || frame.ev === "delta" || frame.ev === "snapshot.begin") && activeSnapshot) throw Error("frame interleaves snapshot")
      if (frame.ev === "snapshot.begin") {
        if (!Number.isSafeInteger(frame.count) || frame.count < 0) throw Error("invalid snapshot count")
        s.snapshots[snapshotKey] = {rows:[], count:frame.count, seen:dictionary()}
      }
      else if (frame.ev === "snapshot.chunk") {
        var snapshot = s.snapshots[snapshotKey]
        if (!snapshot || snapshot.rows.length + frame.items.length > snapshot.count) throw Error("invalid snapshot count")
        frame.items.forEach(function(row) {
          var key = topicKey(topic,row)
          if (snapshot.seen[key]) throw Error("duplicate snapshot key")
          snapshot.seen[key] = true; snapshot.rows.push(row)
        })
      } else if (frame.ev === "snapshot.end") {
        var complete = s.snapshots[snapshotKey]
        if (!complete || complete.rows.length !== complete.count) throw Error("incomplete snapshot")
        if (topic === "torrents") replaceTorrents(complete.rows)
        else {
          var keyed = dictionary()
          complete.rows.forEach(function(row) { keyed[topicKey(topic,row)] = row })
          s.topics[topic] = keyed
          if (topic === "events") complete.rows.forEach(event)
        }
        delete s.snapshots[snapshotKey]
      } else if (frame.ev === "state") {
        s.topics[topic] = frame.data
        if (topic === "summary" || topic === "network" || topic === "settings") s[topic] = frame.data
      } else if (frame.ev === "delta") {
        if (s.snapshots[snapshotKey]) throw Error("delta interleaves snapshot")
        if(topic==="log" && ((frame.removed || []).length || Object.keys(frame.changed || {}).length))throw Error("log topic is append-only")
        if (topic === "torrents") {
          ;(frame.added || []).forEach(function(row) { upsert(row, false) })
          ;(frame.removed || []).forEach(remove)
          Object.keys(frame.changed || {}).forEach(function(hash) {
            if (s.torrents[hash]) {
              var patch = dictionary(); patch.hash = hash
              Object.keys(frame.changed[hash]).forEach(function(field) {
                if (!safeField(field) || (field === "hash" && frame.changed[hash][field] !== hash)) throw Error("immutable or unsafe torrent field")
                patch[field] = frame.changed[hash][field]
              })
              upsert(patch, true)
            }
          })
        } else {
          var target = s.topics[topic] || dictionary()
          ;(frame.added || []).forEach(function(row) { target[topicKey(topic,row)] = row; if (topic === "events") event(row) })
          ;(frame.removed || []).forEach(function(key) { delete target[key] })
          Object.keys(frame.changed || {}).forEach(function(key) {
            if (!target[key]) return
            Object.keys(frame.changed[key]).forEach(function(field) {
              if (!safeField(field)) throw Error("invalid field")
              if (frame.changed[key][field] === null) delete target[key][field]
              else target[key][field] = frame.changed[key][field]
            })
          })
          s.topics[topic] = target
        }
      }
      if(topic==="log")trimLog()
      badge(topic)
      return s.changes.length > 0
    }
    var call = request || s.calls[frame.id]
    delete s.calls[frame.id]
    if (!call) return false
    if (!frame.ok) {
      s.lastError = frame.error
      if (call.ref) keepMutation(call.ref,{op:call.op,status:"failed",error:frame.error})
      return false
    }
    var result = frame.result
    if (call.op === "hello") {
      s.proto = result.proto; s.compatible = result.proto === 1
      if (s.instance && s.instance !== result.instance) {
        replaceTorrents([]); s.subscriptions = dictionary(); s.snapshots = dictionary(); s.topics = dictionary()
        s.hasTorrents = false; s.badges={}
        s.events = []; s.eventSeen = dictionary(); s.summary = {}; s.network = {}; s.settings = {}
      }
      s.subscriptions = dictionary(); s.snapshots = dictionary()
      s.instance = result.instance; s.info = result
    } else if (call.op === "app.info") s.info = result
    else if (call.op === "app.network") s.network = result.state
    else if(call.op==="app.settings.get" || call.op==="app.settings.set")s.settings=result.settings || result
    else if (call.op === "transfer.info") s.transfer = result
    else if(call.op==="app.log"){
      var log=s.topics.log || dictionary();result.entries.forEach(function(r){log[String(r.seq)]=r});s.topics.log=log;trimLog()
    }
    else if(call.op==="shows.list")domainList("shows",result.shows)
    else if(call.op==="rss.feeds")domainList("rss.feeds",result.feeds)
    else if (call.op === "torrents.list") replaceTorrents(result.torrents)
    else if (call.op === "torrents.get") upsert(result.torrent, false)
    else if(call.op==="torrents.trackers")domainList("torrent:"+call.args.hash+".trackers",result.trackers)
    else if (call.op === "torrents.peers") {
      var peers = dictionary()
      result.peers.forEach(function(peer) { peers[peer.addr] = peer })
      s.topics["torrent:" + call.args.hash + ".peers"] = peers
    }
    else if (call.op === "torrents.files") {
      var files = dictionary()
      result.files.forEach(function(file) { files[String(file.index)] = file })
      s.topics["torrent:" + call.args.hash + ".files"] = files
    }
    else if (call.op === "subscribe") s.subscriptions[result.sub] = {topics:result.topics, rev:0, resync:false}
    else if (call.op === "unsubscribe") {
      delete s.subscriptions[call.args.sub]
      Object.keys(s.snapshots).forEach(function(key) { if (key.indexOf(call.args.sub + ":") === 0) delete s.snapshots[key] })
    } else if (call.op === "imdb.lookup" && result.card) {
      var ids = Object.keys(s.cards)
      if (ids.length >= 128) delete s.cards[ids[0]]
      s.cards[result.card.id] = result.card
    }
    if (call.ref) keepMutation(call.ref,{op:call.op,status:result.state === "pending" ? "pending" : "done",result:result})
    return s.changes.length > 0
  }
  s.rows = function(order) {
    if (order) return order.map(function(hash) { return s.torrents[hash] })
    if (s.keysDirty) { s.keys = Object.keys(s.torrents); s.keysDirty = false }
    return s.keys.map(function(hash) { return s.torrents[hash] })
  }
  s.filter = function(state) {
    if (s.keysDirty) { s.keys = Object.keys(s.torrents); s.keysDirty = false }
    return state === "all" ? s.keys.slice() : s.keys.filter(function(hash) { return s.torrents[hash].state === state })
  }
  s.sort = function(key, reverse) {
    key = String(key || "added").toLowerCase(); reverse = !!reverse
    if (key === "↓" || key === "dl_bps") key = "rate"
    if (["added","name","progress","rate","size","state","eta","up","peers","ratio"].indexOf(key) < 0) key = "added"
    var cached = s.sortCache[key + reverse], current = clock()
    if (cached && cached.membership === s.membershipVersion &&
        (cached.version === s.sortVersions[key] || ((key === "rate" || key === "up") && current - cached.at < 1000))) return cached.keys
    var property = {added:"added_at", name:"name", progress:"progress", rate:"dl_bps", size:"size", state:"state", eta:"eta_s",up:"ul_bps",peers:"peers",ratio:"ratio"}[key] || "added_at"
    var descending = ["added", "progress", "rate", "size", "up", "peers", "ratio"].indexOf(key) >= 0
    var entries = s.filter("all").map(function(hash) { return s.sortValues[hash] })
    var direction=descending!==reverse ? -1 : 1
    entries.sort(key==="name" ? (reverse ? compareNamesReverse : compareNames) : function(a,b) {
      var left = a[property], right = b[property]
      if (left === undefined || right === undefined) {
        if(left!==undefined)return -1
        if(right!==undefined)return 1
      } else {
        var compared = left < right ? -1 : left > right ? 1 : 0
        if(compared)return compared*direction
      }
      return a.hash<b.hash ? -1 : a.hash>b.hash ? 1 : 0
    })
    var hashes = entries.map(function(entry) { return entry.hash })
    s.sortCache[key + reverse] = {keys:hashes, membership:s.membershipVersion, version:s.sortVersions[key], at:current}
    return hashes
  }
  // Qt's model owns stable delegates. Only changed roles emit row updates.
  var mirror = {model:null, indexes:dictionary(), order:[], generation:-1, sorted:null}
  s.syncModel = function(model, key, reverse, reorder) {
    var structural = false
    if (mirror.model !== model) {
      mirror = {model:model, indexes:dictionary(), order:[], generation:-1, sorted:null}
      if (model.count) model.remove(0, model.count)
    }
    function reindex(from) {
      for (var index = from; index < mirror.order.length; ++index) mirror.indexes[mirror.order[index]] = index
    }
    if (mirror.generation !== s.generation) {
      s.changes.forEach(function(change) {
        var index = mirror.indexes[change.hash], row = s.torrents[change.hash]
        if (!row) {
          if (index !== undefined) {
            model.remove(index); mirror.order.splice(index,1); delete mirror.indexes[change.hash]; reindex(index)
            structural = true
          }
        } else if (index === undefined) {
          mirror.indexes[change.hash] = mirror.order.length; mirror.order.push(change.hash)
          model.append(Object.assign({},row,{uiReady:ready(row),uiName:s.sortNames[change.hash],uiVisible:s.matches(row.state,ready(row),s.sortNames[change.hash],s.filterState,s.filterText)})); structural = true
        } else {
          change.fields.forEach(function(field) { model.setProperty(index,field,row[field] === undefined ? null : row[field]) })
          if(change.fields.some(function(field){return ["playable","name","state"].indexOf(field)>=0}))model.setProperty(index,"uiVisible",s.matches(row.state,ready(row),s.sortNames[change.hash],s.filterState,s.filterText))
          if(change.fields.indexOf("playable")>=0)model.setProperty(index,"uiReady",ready(row))
          if(change.fields.indexOf("name")>=0)model.setProperty(index,"uiName",s.sortNames[change.hash])
          if (change.fields.indexOf("state") >= 0 || change.fields.indexOf("staged") >= 0) structural = true
        }
      })
      mirror.generation = s.generation
    }
    var ordered = s.sort(key, reverse)
    if (mirror.sorted !== ordered) {
      var moving=false
      for (var i=0; i<ordered.length; ++i) {
        var from = mirror.indexes[ordered[i]]
        if (from !== i) {
          if(!moving){moving=true;if(reorder)reorder(true)}
          model.move(from,i,1); mirror.order.splice(i,0,mirror.order.splice(from,1)[0]); reindex(i)
          structural = true
        }
      }
      mirror.sorted = ordered
      if(moving && reorder)reorder(false)
    }
    return structural
  }
  s.filterState="all";s.filterText=""
  s.setFilter=function(state,text) {
    text=String(text || "").toLocaleLowerCase()
    if(state===s.filterState && text===s.filterText)return
    s.filterState=state;s.filterText=text
    if(!mirror.model)return
    mirror.order.forEach(function(hash,index){var row=s.torrents[hash];mirror.model.setProperty(index,"uiVisible",s.matches(row.state,ready(row),s.sortNames[hash],state,text))})
  }
  s.pendingByHash=dictionary();s.failedByHash=dictionary()
  s.syncActions=function(actions) {
    var pending=dictionary(),failed=dictionary()
    Object.keys(actions).forEach(function(ref) {
      var action=actions[ref],args=action.args || {},hashes=args.hashes || (args.hash ? [args.hash] : [])
      hashes.forEach(function(hash) {
        if(action.status==="pending")pending[hash]=true
        else if(action.status==="failed" || action.status==="error" || action.status==="no answer")failed[hash]=action.error || {message:"no answer"}
      })
    })
    s.pendingByHash=pending;s.failedByHash=failed
  }
  s.clearFailures=function(actions) {
    Object.keys(actions).forEach(function(ref) {if(actions[ref].status!=="pending")delete actions[ref]})
    s.failedByHash=dictionary()
  }
  s.visibleKeys=function(filter,text,key,reverse) {
    text=String(text || "").toLocaleLowerCase()
    return s.sort(key,reverse).filter(function(hash){var row=s.torrents[hash];return s.matches(row.state,ready(row),s.sortNames[hash],filter,text)})
  }
  s.selectRange=function(order,anchor,target,selected) {
    var result=Object.assign({},selected),from=order.indexOf(anchor),to=order.indexOf(target)
    if(from<0)from=to
    for(var i=Math.min(from,to);i<=Math.max(from,to);i++)if(i>=0)result[order[i]]=true
    return result
  }
  s.modelIndex = function(hash) {return mirror.indexes[hash]===undefined ? -1 : mirror.indexes[hash]}
  s.matches = function(state,ready,name,filter,text) {
    var include=filter==="all" || filter===state || (filter==="downloading" && ["downloading","stalled","metadata","checking"].indexOf(state)>=0) || (filter==="ready" && ready)
    return include && (!text || name.indexOf(text)>=0)
  }
  s.modelOrder = function() { return mirror.order }
  s.model = function(keys) {
    var result = {}
    keys.forEach(function(key) {
      if (key === "subscriptions") {
        result.subscriptions = {}
        Object.keys(s.subscriptions).forEach(function(sub) {
          var value = s.subscriptions[sub]
          result.subscriptions[sub] = {topic:value.topics[0], rev:value.rev, resync:value.resync}
        })
      } else result[key] = s[key]
    })
    return result
  }
  return s
}
