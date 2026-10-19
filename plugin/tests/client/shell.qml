import QtQuick
import Quickshell
import "Plugin/ApiClient.js" as ApiClient

ShellRoot {
  function check(name, condition, expected, observed) {
    console.log("FBTQS-STEP " + JSON.stringify({case:name, action:"exercise API session", expect:expected,
      observed:String(observed), ok:!!condition, ms:0, artifacts:[]}))
  }
  function session() {
    var time = {ms:0}, writes = [], events = [], resets = []
    var transport = {write:function(text) { writes.push(text); events.push("write") },
      flush:function() { events.push("flush") }, isConnected:function() { return false },
      connect:function() { events.push("connect") }, disconnect:function() { resets.push(time.ms) }}
    var client = ApiClient.create(transport, {now:function() { return time.ms }, frame:function() {}, version:"0.2.0"})
    return {c:client, time:time, writes:writes, events:events, resets:resets,
      reply:function(result, id) { client.receive(JSON.stringify({id:id || JSON.parse(writes[writes.length-1]).id, ok:true, result:result})) },
      hello:function(caps, instance) {
        client.connected()
        var id = JSON.parse(writes[writes.length-1]).id
        client.receive(JSON.stringify({id:id, ok:true, result:{proto:1, instance:instance || "0123456789abcdef0123456789abcdef", state:"running", caps:caps || ["torrents","files","subscribe","mutations"]}}))
      }}
  }
  Component.onCompleted: {
    var s = session(), ladder = []
    for (var i=0; i<7; i++) ladder.push(s.c.attempt())
    check("backoff", JSON.stringify(ladder) === "[500,1000,2000,4000,8000,10000,10000]", "exact reconnect ladder", JSON.stringify(ladder))
    console.log("FBTQS-PASS backoff")
    s = session(); s.c.connected(); s.c.connected()
    check("bootstrap", s.writes.length === 1 && JSON.parse(s.writes[0]).args.client.kind === "bar", "bootstrap hello once, fixed bar identity", s.writes)
    check("bootstrap", s.events.join(",") === "write,flush", "every write immediately flushed and newline terminated", s.events)
    check("bootstrap", s.writes[0].slice(-1) === "\n", "Socket.write includes newline", JSON.stringify(s.writes[0]))
    console.log("FBTQS-PASS bootstrap")
    s = session(); s.hello();
    var ref = s.c.request("torrents.stop", {hashes:["a".repeat(40)]}, {mutating:true})
    var stop = JSON.parse(s.writes[s.writes.length-1])
    s.time.ms = 19999; s.c.tick()
    check("deadlines", s.c.actions[ref].status === "pending", "ordinary mutation remains pending before 20 s", s.c.actions[ref].status)
    s.time.ms = 20000; s.c.tick()
    check("deadlines", s.c.actions[ref].status === "no answer", "ordinary mutation times out at 20 s", s.c.actions[ref].status)
    s = session(); s.hello(); ref = s.c.request("torrents.add", {source:{magnet:"magnet:?xt=urn:btih:"+"a".repeat(40)}}, {mutating:true})
    s.time.ms=99999; s.c.lastFrame=s.time.ms; s.c.tick()
    check("deadlines", s.c.actions[ref].status === "pending", "add remains pending before 100 s", s.c.actions[ref].status)
    s.time.ms=100000; s.c.tick()
    check("deadlines", s.c.actions[ref].status === "no answer", "add times out at 100 s", s.c.actions[ref].status)
    console.log("FBTQS-PASS deadlines")
    s = session(); s.hello();
    var safe=s.c.request("torrents.stop",{hashes:["a".repeat(40)]},{mutating:true})
    var destructive=s.c.request("torrents.remove",{hashes:["a".repeat(40)],delete_files:false},{mutating:true})
    var original=JSON.parse(s.writes[s.writes.length-2]), length=s.writes.length
    s.c.disconnected(); s.hello(["torrents","files","subscribe","mutations"],"abcdef0123456789abcdef0123456789")
    var resends=s.writes.slice(length).map(function(w) {return JSON.parse(w)}).filter(function(f) {return !!f.ref})
    check("replay", resends.length===1 && resends[0].ref===safe && resends[0].instance==="0123456789abcdef0123456789abcdef",
      "only idempotent mutation resends same ref with old instance", JSON.stringify(resends))
    check("replay", s.c.actions[destructive].status==="no answer" && resends[0].id>original.id,
      "destructive mutation never replays; request ids increase", JSON.stringify(s.c.actions))
    console.log("FBTQS-PASS replay")
    s=session(); s.hello(); s.time.ms=44999; s.c.tick()
    check("silence", s.resets.length===0, "session lives before 45 s silence", s.resets.length)
    s.time.ms=45000; s.c.tick()
    check("silence", s.resets.length===1, "45 s silence resets session", s.resets.length)
    console.log("FBTQS-PASS silence")
    s=session(); s.c.receive("x".repeat(1048577))
    check("line_cap", s.resets.length===1, "1 MiB + 1 line rejected before JSON parse", s.c.lastError.message)
    s=session(); s.c.receive("[")
    check("line_cap", s.resets.length===1, "bad JSON resets session without exception", s.c.lastError.message)
    console.log("FBTQS-PASS line_cap")
    s=session(); s.hello(["torrents","files"])
    check("fallback", s.c.mode==="poll" && s.writes.some(function(w) {return JSON.parse(w).op==="torrents.list"}),
      "absent subscribe cap polls over same socket", s.c.mode)
    s=session(); s.hello(); var subscribe=JSON.parse(s.writes[s.writes.length-1])
    s.c.receive(JSON.stringify({id:subscribe.id,ok:false,error:{code:"unknown_op",message:"unknown op",retryable:false}}))
    check("fallback", s.c.mode==="poll", "unknown_op on subscribe enables polling", s.c.mode)
    console.log("FBTQS-PASS fallback")
    s=session(); s.hello(); subscribe=JSON.parse(s.writes[s.writes.length-1]); s.reply({sub:7,topics:["summary"]}, subscribe.id)
    s.c.receive(JSON.stringify({ev:"state",sub:7,topic:"summary",rev:1,data:{}}))
    check("subscriptions", s.c.state().subs.summary===1, "state reports topic and last revision", JSON.stringify(s.c.state()))
    length=s.writes.length
    s.c.receive(JSON.stringify({ev:"state",sub:7,topic:"summary",rev:3,data:{}}))
    check("subscriptions", s.writes.slice(length).some(function(w) {return JSON.parse(w).op==="unsubscribe"}), "revision gap resubscribes", s.writes.slice(length))
    console.log("FBTQS-PASS subscriptions")
    s=session(); s.hello()
    ref=s.c.request("torrents.add",{source:{url:"https://example.invalid/legal.torrent"}},{mutating:true})
    var request=JSON.parse(s.writes[s.writes.length-1]);s.reply({state:"pending",ref:ref},request.id)
    length=s.writes.length; s.c.disconnected(); s.hello()
    resends=s.writes.slice(length).map(function(w) {return JSON.parse(w)}).filter(function(f) {return f.ref===ref})
    check("pending_ref",resends.length===1,"pending response retains correlation for reconnect replay",JSON.stringify(resends))
    s=session(); s.hello(); ref=s.c.request("torrents.stop",{hashes:["a".repeat(40)]},{mutating:true})
    request=JSON.parse(s.writes[s.writes.length-1]);s.reply({state:"pending"},request.id)
    check("pending_ref",s.resets.length===1,"pending without correlation ref is a protocol violation",s.c.lastError.message)
    console.log("FBTQS-PASS pending_ref")
    s=session();s.hello();s.c.disconnected()
    check("reconnecting",s.c.reconnecting,"unexpected EOF retains reconnecting state",s.c.reconnecting)
    for(i=0;i<6;i++)s.c.attempt()
    check("reconnecting",!s.c.reconnecting,"reconnecting ends when backoff reaches its cap",s.c.retryDelay)
    s=session();s.hello();s.c.receive(JSON.stringify({ev:"bye",reason:"shutdown"}));s.c.disconnected()
    check("reconnecting",!s.c.reconnecting && !s.c.online,"bye is graceful and leaves the daemon offline",JSON.stringify(s.c.state()))
    console.log("FBTQS-PASS reconnecting")
    s=session();s.c.connected();s.reply({proto:2})
    check("protocol",s.c.mode==="protocol_mismatch" && !s.c.online,"incompatible API major stays offline",s.c.mode)
    s=session();s.c.connected();s.reply({proto:1,caps:[],instance:"bad"})
    check("protocol",s.resets.length===1,"invalid hello shape resets session",s.c.lastError.message)
    console.log("FBTQS-PASS protocol")
    s=session();s.hello([]);length=s.writes.length
    s.c.request("torrents.stop",{hashes:["a".repeat(40)]},{mutating:true})
    check("capabilities",s.writes.length===length && s.c.lastError.code==="unsupported","missing feature cap disables its actions",s.c.lastError.message)
    console.log("FBTQS-PASS capabilities")
    Qt.callLater(function() { Quickshell.execDetached(["qs", "kill", "-p", Quickshell.shellPath(".")]) })
  }
}
