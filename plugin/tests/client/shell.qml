import QtQuick
import Quickshell
import "Plugin/ApiClient.js" as ApiClient

ShellRoot {
  function check(name, condition, expected, observed) {
    console.log("FBTQS-STEP " + JSON.stringify({case:name, action:"exercise API session", expect:expected,
      observed:String(observed), ok:!!condition, ms:0, artifacts:[]}))
  }
  function session() {
    var time = {ms:0}, writes = [], events = [], resets = [], recovery = []
    var transport = {write:function(text) { writes.push(text); events.push("write") },
      flush:function() { events.push("flush") }, isConnected:function() { return false },
      connect:function() { events.push("connect") }, disconnect:function(recover) { resets.push(time.ms);recovery.push(recover===true) }}
    var client = ApiClient.create(transport, {now:function() { return time.ms }, frame:function() {}, version:"0.2.0"})
    return {c:client, time:time, writes:writes, events:events, resets:resets, recovery:recovery,
      reply:function(result, id) { client.receive(JSON.stringify({id:id || JSON.parse(writes[writes.length-1]).id, ok:true, result:result})) },
      hello:function(caps, instance) {
        client.connected()
        var id = JSON.parse(writes[writes.length-1]).id
        client.receive(JSON.stringify({id:id, ok:true, result:{proto:1, instance:instance || "0123456789abcdef0123456789abcdef", state:"running", caps:caps || ["torrents","files","subscribe","mutations"]}}))
      }}
  }
  Component.onCompleted: {
    var s = session(), i
    s.c.attempt();s.c.attempt()
    check("socket_wakeup",s.events.join(",")==="connect" && s.c.connecting,"one explicit wake makes one connection attempt",s.events)
    s.c.disconnected();s.time.ms=60000;s.c.tick()
    check("socket_wakeup",s.events.join(",")==="connect" && s.c.mode==="stopped","clock ticks never reconnect while waiting for socket appearance",s.events)
    s.c.attempt()
    check("socket_wakeup",s.events.join(",")==="connect,connect","a new socket/input wake retries immediately",s.events)
    console.log("FBTQS-PASS socket_wakeup")
    s = session(); s.c.connected(); s.c.connected()
    check("bootstrap", s.writes.length === 1 && JSON.parse(s.writes[0]).args.client.kind === "bar", "bootstrap hello once, fixed bar identity", s.writes)
    check("bootstrap", s.events.join(",") === "write,flush", "every write immediately flushed and newline terminated", s.events)
    check("bootstrap", s.writes[0].slice(-1) === "\n", "Socket.write includes newline", JSON.stringify(s.writes[0]))
    console.log("FBTQS-PASS bootstrap")
    s=session();s.hello();s.c.disconnected()
    check("transport_loss",s.c.lastError===null,"interrupted automatic presence/subscription requests use reconnect state, not a sticky action error",JSON.stringify(s.c.lastError))
    s=session();s.hello();s.c.request("app.info");s.c.disconnected()
    check("transport_loss",!s.c.lastError,"disconnect clears the global banner after reporting the interrupted request via its callback",JSON.stringify(s.c.lastError))
    s=session();s.hello();var uncertain=s.c.request("torrents.remove",{hashes:["a".repeat(40)],delete_files:false},{mutating:true,internal:true});s.c.disconnected()
    check("transport_loss",s.c.actions[uncertain].status==="no answer" && !s.c.lastError,"internal setting never suppresses uncertainty for a user mutation",JSON.stringify(s.c.actions[uncertain]))
    console.log("FBTQS-PASS transport_loss")
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
    check("deadlines",s.c.actions[ref].args.source.magnet!==undefined,"failed action retains its row/source correlation",JSON.stringify(s.c.actions[ref]))
    console.log("FBTQS-PASS deadlines")
    s=session();s.hello();s.c.request("app.info")
    s.time.ms=20000;s.c.tick()
    check("error_recovery",s.c.lastError && s.c.lastError.code==="timeout","unanswered explicit request exposes its typed timeout",s.c.lastError)
    var recovery=s.c.request("app.network");s.reply({state:{ready:true,message:"Binding off"}},recovery)
    check("error_recovery",!s.c.lastError,"later successful explicit operation clears an earlier timeout",s.c.lastError)
    s.c.lastError={code:"timeout",message:"old timeout",retryable:true};s.c.disconnected()
    check("error_recovery",!s.c.lastError,"connection reset clears the stale global error while action outcomes remain recorded",s.c.lastError)
    s.c.lastError={code:"timeout",message:"old timeout",retryable:true};s.hello()
    check("error_recovery",!s.c.lastError,"new connection hello clears old errors",s.c.lastError)
    s.c.lastError={code:"timeout",message:"old timeout",retryable:true};s.c.receive(JSON.stringify({ev:"bye",reason:"shutdown"}))
    check("error_recovery",!s.c.lastError,"graceful stop removes a stale timeout",s.c.lastError)
    console.log("FBTQS-PASS error_recovery")
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
    s=session();s.hello();var renameRef=s.c.request("torrents.rename",{hash:"a".repeat(40),name:"Personal title"},{mutating:true})
    length=s.writes.length;s.c.disconnected();s.hello()
    resends=s.writes.slice(length).map(function(w){return JSON.parse(w)}).filter(function(f){return f.op==="torrents.rename"})
    check("replay",resends.length===1 && resends[0].ref===renameRef && resends[0].args.name==="Personal title","display rename retains its idempotent intent after a dropped reply",JSON.stringify(resends))
    console.log("FBTQS-PASS replay")
    s=session(); s.hello(); s.time.ms=44999; s.c.tick()
    check("silence", s.resets.length===0, "session lives before 45 s silence", s.resets.length)
    s.time.ms=45000; s.c.tick()
    check("silence", s.resets.length===1, "45 s silence resets session", s.resets.length)
    check("silence",s.recovery[0]===true,"silence requests reconnection even when the socket inode is unchanged",s.recovery)
    s.time.ms=1800000;s.c.tick()
    check("silence",s.resets.length===1,"a long pause does not replay a storm of missed timeout ticks",s.resets.length)
    console.log("FBTQS-PASS silence")
    s=session();s.hello();s.c.receive(JSON.stringify({ev:"bye",reason:"hello_required"}))
    check("resume_hello",!s.c.graceful && s.c.reconnecting && s.recovery[0]===true,
      "expired hello reconnects instead of announcing that the daemon stopped",JSON.stringify(s.c.state()))
    check("resume_hello",s.c.lastError.code==="timeout" && s.c.lastError.retryable,"hello expiry remains a transient timeout",JSON.stringify(s.c.lastError))
    s.c.receive(JSON.stringify({ev:"bye",reason:"shutdown"}))
    check("resume_hello",s.c.graceful && !s.c.reconnecting,"explicit shutdown still stops recovery",JSON.stringify(s.c.state()))
    console.log("FBTQS-PASS resume_hello")
    s=session(); s.c.receive("x".repeat(1048577))
    check("line_cap", s.resets.length===1, "1 MiB + 1 line rejected before JSON parse", s.c.lastError.message)
    s=session(); s.c.receive("[")
    check("line_cap", s.resets.length===1, "bad JSON resets session without exception", s.c.lastError.message)
    check("line_cap",s.recovery[0]===false,"invalid protocol does not request an automatic retry loop",s.recovery)
    console.log("FBTQS-PASS line_cap")
    s=session();s.hello();var subscribe=JSON.parse(s.writes[s.writes.length-1])
    s.c.receive(JSON.stringify({id:subscribe.id,ok:false,error:{code:"unknown_op",message:"unknown op",retryable:false}}))
    check("subscriptions_required",s.resets.length===1 && !s.c.online && !s.writes.some(function(w){return ["torrents.list","transfer.info","app.network"].indexOf(JSON.parse(w).op)>=0}),"subscription failure resets the connection without silently polling old operations",s.writes)
    console.log("FBTQS-PASS subscriptions_required")
    s=session(); s.hello(); subscribe=JSON.parse(s.writes[s.writes.length-1]); s.reply({sub:7,topics:["summary"]}, subscribe.id)
    s.c.receive(JSON.stringify({ev:"state",sub:7,topic:"summary",rev:1,data:{}}))
    check("subscriptions", s.c.state().subs.summary===1, "state reports topic and last revision", JSON.stringify(s.c.state()))
    length=s.writes.length
    s.c.receive(JSON.stringify({ev:"state",sub:7,topic:"summary",rev:3,data:{}}))
    check("subscriptions", s.writes.slice(length).some(function(w) {return JSON.parse(w).op==="unsubscribe"}), "revision gap resubscribes", s.writes.slice(length))
    console.log("FBTQS-PASS subscriptions")
    s=session();s.hello();var firstSubscribe=JSON.parse(s.writes[s.writes.length-1])
    s.c.visibility(false,true,["torrent:"+"a".repeat(40)+".files"])
    s.reply({sub:91,topics:["summary"]},firstSubscribe.id)
    check("detail_subscriptions",!s.c.subs[91] && s.writes.some(function(w){var f=JSON.parse(w);return f.op==="unsubscribe" && f.args.sub===91}),"late subscription reply is immediately unsubscribed after visibility generation changes",s.writes)
    // Retire the late group before admitting the latest base/detail pair.
    var retirement=s.writes.map(function(w){return JSON.parse(w)}).filter(function(f){return f.op==="unsubscribe" && f.args.sub===91}).slice(-1)[0]
    s.reply({state:"done"},retirement.id)
    var baseRequest=s.writes.map(function(w){return JSON.parse(w)}).filter(function(f){return f.op==="subscribe" && s.c.pending[f.id]}).slice(-1)[0]
    s.reply({sub:92,topics:baseRequest.args.topics},baseRequest.id)
    var requested=s.writes.map(function(w){return JSON.parse(w)}).filter(function(f){return f.op==="subscribe"})
    check("detail_subscriptions",requested[requested.length-1].args.topics.join(",")==="torrent:"+"a".repeat(40)+".files","each selected detail topic has its own subscribe call",requested)
    console.log("FBTQS-PASS detail_subscriptions")
    s=session();s.c.visibility(false,true,[]);s.hello()
    var nextSub=1000
    function settleSubscriptions(){
      for(var attempt=0;attempt<64;attempt++){
        var ids=Object.keys(s.c.pending),entry=ids.length ? s.c.pending[ids[0]] : null
        if(!entry)return
        if(entry.op==="subscribe")s.reply({sub:nextSub++,topics:entry.args.topics},entry.id)
        else s.reply({state:"done"},entry.id)
      }
      throw Error("subscription reconciliation failed to settle")
    }
    settleSubscriptions()
    var baseId=Number(Object.keys(s.c.subs)[0]);length=s.writes.length
    s.c.receive(JSON.stringify({ev:"state",sub:baseId,topic:"summary",rev:1,data:{}}))
    s.c.visibility(false,true,["torrent:"+"a".repeat(40)+".files"]);settleSubscriptions()
    s.c.visibility(false,true,["torrent:"+"a".repeat(40)+".peers"]);settleSubscriptions()
    var edits=s.writes.slice(length).map(function(w){return JSON.parse(w)})
    check("subscription_reuse",s.c.subs[baseId] && s.c.subs[baseId].rev===1 && s.c.topicRevs.summary===1,"detail changes preserve the base subscription and revision",JSON.stringify(s.c.subs))
    check("subscription_reuse",edits.filter(function(f){return f.op==="subscribe"}).every(function(f){return f.args.topics.length===1 && f.args.topics[0].indexOf("torrent:")===0}),"switching Files to Peers never requests another full torrents snapshot",JSON.stringify(edits))
    check("subscription_reuse",!edits.some(function(f){return f.op==="app.client"}),"detail changes do not resend unchanged client presence",JSON.stringify(edits))
    check("subscription_reuse",Object.keys(s.c.subs).length===2 && Object.values(s.c.subs).some(function(sub){return sub.topics[0].endsWith(".peers")}),"retired detail is released before its replacement; no subscription leak",JSON.stringify(s.c.subs))
    s.c.subscribeAll(true);settleSubscriptions()
    check("subscription_reuse",!s.c.subs[baseId],"explicit Refresh still replaces base snapshots",JSON.stringify(s.c.subs))
    s.c.visibility(false,false,[]);settleSubscriptions()
    check("subscription_reuse",Object.keys(s.c.subs).length===1 && Object.values(s.c.subs)[0].interval===5000 && Object.values(s.c.subs)[0].topics.join(",")==="summary","closing the UI retires detail groups and restores the bar's slower interval",JSON.stringify(s.c.subs))
    console.log("FBTQS-PASS subscription_reuse")
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
    s=session();s.hello()
    Object.keys(s.c.pending).forEach(function(id){var entry=s.c.pending[id];s.reply(entry.op==="subscribe" ? {sub:77,topics:entry.args.topics} : {},Number(id))})
    ref=s.c.request("torrents.recheck",{hashes:["a".repeat(40)]},{mutating:true})
    request=JSON.parse(s.writes[s.writes.length-1]);s.reply({state:"pending",ref:ref},request.id)
    s.time.ms=21000;s.c.receive(JSON.stringify({ev:"state",topic:"summary",data:{}}));s.c.tick()
    check("recheck_progress",s.c.actions[ref].status==="pending" && !s.c.lastError && !!s.c.awaiting[ref],"accepted recheck remains in progress beyond the ordinary reply deadline",JSON.stringify(s.c.actions[ref]))
    s.time.ms=120000;s.c.receive(JSON.stringify({ev:"state",topic:"summary",data:{}}));s.c.tick()
    check("recheck_progress",s.c.actions[ref].status==="pending" && !s.c.lastError,"heartbeats keep a long accepted check pending without a false timeout",JSON.stringify(s.c.actions[ref]))
    s.c.receive(JSON.stringify({ev:"mutation.result",op:"torrents.recheck",ref:ref,outcome:"done",result:{rechecked:["a".repeat(40)],failed:[]}}))
    check("recheck_progress",s.c.actions[ref].status==="done" && !s.c.awaiting[ref] && !s.c.lastError,"final receipt resolves checking without leaving a global error",JSON.stringify(s.c.actions[ref]))
    s=session();s.hello();ref=s.c.request("torrents.recheck",{hashes:["a".repeat(40)]},{mutating:true})
    s.c.disconnected()
    check("recheck_progress",s.c.actions[ref].status==="no answer" && !s.c.lastError,"a genuinely lost connection still reports the uncertain recheck outcome",JSON.stringify(s.c.actions[ref]))
    s.c.receive(JSON.stringify({ev:"mutation.result",op:"torrents.recheck",ref:ref,outcome:"done",result:{rechecked:["a".repeat(40)],failed:[]}}))
    check("recheck_progress",s.c.actions[ref].status==="done" && !s.c.lastError,"a late confirmed result clears only that action's stale no-answer error",JSON.stringify(s.c.actions[ref]))
    s=session();s.hello();ref=s.c.request("torrents.recheck",{hashes:["a".repeat(40)]},{mutating:true});s.c.disconnected()
    s.c.lastError={code:"upstream",message:"another operation failed",retryable:true}
    s.c.receive(JSON.stringify({ev:"mutation.result",op:"torrents.recheck",ref:ref,outcome:"done",result:{rechecked:["a".repeat(40)],failed:[]}}))
    check("recheck_progress",s.c.lastError && s.c.lastError.message==="another operation failed","a recheck receipt preserves an unrelated operation's error",s.c.lastError)
    console.log("FBTQS-PASS recheck_progress")
    ;["torrents.remove","torrents.add"].forEach(function(op){
      s=session();s.c.visibility(false,true,[]);s.hello()
      var args=op==="torrents.remove" ? {hashes:["a".repeat(40)],delete_files:false} : {source:{restore:"a".repeat(40)}}
      var ref=s.c.request(op,args,{mutating:true}),request=JSON.parse(s.writes[s.writes.length-1])
      s.reply({state:"pending",ref:ref},request.id)
      s.time.ms=21000;s.c.receive(JSON.stringify({ev:"state",topic:"summary",data:{}}));s.c.tick()
      check("cleanup_progress",s.c.actions[ref].status==="pending" && !s.c.lastError,"accepted removal/Undo remains pending past the ordinary reply deadline",s.c.actions[ref])
      s.c.disconnected()
      // Remove is never replayed; its outcome comes from the live receipt or
      // event history. Do not clear any unrelated error during reconciliation.
      var failure={code:"timeout",message:"no answer",retryable:true}
      s.c.actions[ref].error=failure;s.c.lastError=failure
      s.c.subs[801]={topics:["events"],rev:0}
      s.c.receive(JSON.stringify({ev:"snapshot.chunk",topic:"events",sub:801,rev:1,items:[{ev:"mutation.result",op:op,ref:ref,outcome:"done",result:{state:"done"}}]}))
      check("cleanup_progress",s.c.actions[ref].status==="done" && !s.c.lastError,"late event-history success clears the correlated timeout",s.c.actions[ref])
      s.c.lastError={code:"upstream",message:"unrelated read failed",retryable:true}
      s.c.receive(JSON.stringify({ev:"mutation.result",op:op,ref:ref,outcome:"done",result:{state:"done"}}))
      check("cleanup_progress",s.c.lastError.message==="unrelated read failed","cleanup success preserves an unrelated failure",s.c.lastError)
    })
    console.log("FBTQS-PASS cleanup_progress")
    s=session();s.hello();s.c.disconnected()
    check("reconnecting",!s.c.reconnecting && s.c.mode==="stopped","unexpected EOF waits quietly for a socket/input edge",s.c.state())
    s.c.attempt()
    check("reconnecting",s.c.reconnecting && s.c.connecting,"socket appearance begins a real reconnect",s.c.state())
    s=session();s.hello();s.c.receive(JSON.stringify({ev:"bye",reason:"shutdown"}));s.c.disconnected()
    check("reconnecting",!s.c.reconnecting && !s.c.online,"bye is graceful and leaves the daemon offline",JSON.stringify(s.c.state()))
    console.log("FBTQS-PASS reconnecting")
    s=session();s.c.connected();s.reply({proto:2})
    check("protocol",s.c.mode==="protocol_mismatch" && !s.c.online,"incompatible API major stays offline",s.c.mode)
    s=session();s.c.connected();s.reply({proto:1,caps:[],instance:"bad"})
    check("protocol",s.resets.length===1,"invalid hello shape resets session",s.c.lastError.message)
    console.log("FBTQS-PASS protocol")
    s=session();s.hello();var optionalError=null
    var optional=s.c.request("imdb.lookup",{name:"Missing"},{quiet:true,callback:function(result,error){optionalError=error}})
    s.c.receive(JSON.stringify({id:optional,ok:false,error:{code:"upstream",message:"optional poster unavailable",retryable:true}}))
    check("optional_error",optionalError && optionalError.code==="upstream" && !s.c.lastError,"optional read retains its typed callback failure without becoming a global mutation/error banner",optionalError)
    console.log("FBTQS-PASS optional_error")
    s=session();s.hello(["torrents","subscribe","clipboard"])
    check("clipboard_events",s.writes.some(function(w){var f=JSON.parse(w);return f.op==="subscribe" && f.args.topics.indexOf("events")>=0}),"clipboard-capable bar subscribes to live events even with both UIs closed",s.writes)
    console.log("FBTQS-PASS clipboard_events")
    ;["shutdown","idle","restart","invalid_message"].forEach(function(reason){
      s=session();s.hello();ref=s.c.request("app.shutdown",{reason:"user"},{mutating:true})
      request=JSON.parse(s.writes[s.writes.length-1]);s.reply({state:"pending",ref:ref},request.id)
      s.c.receive(JSON.stringify({ev:"bye",reason:reason}));s.c.disconnected()
      check("shutdown_outcome",s.c.actions[ref].status===(reason==="shutdown" ? "confirmed" : "no answer"),"only explicit shutdown bye confirms the pending shutdown",JSON.stringify(s.c.actions[ref]))
    })
    console.log("FBTQS-PASS shutdown_outcome")
    s=session();s.c.visibility(false,true,[]);s.hello(["torrents","subscribe","dashboard_presence"])
    var declarations=s.writes.map(function(w){return JSON.parse(w)}).filter(function(f){return f.op==="app.client"})
    check("dashboard_presence",JSON.parse(s.writes[0]).args.interactive && declarations[0].args.dashboard_visible && declarations[0].args.interactive,
      "already visible dashboard declares presence on connect",JSON.stringify(declarations))
    s.c.visibility(false,false,[])
    declarations=s.writes.map(function(w){return JSON.parse(w)}).filter(function(f){return f.op==="app.client"})
    check("dashboard_presence",declarations[declarations.length-1].args.dashboard_visible===false && declarations[declarations.length-1].args.interactive===false,
      "hiding dashboard clears shared-socket presence",JSON.stringify(declarations))
    s=session();s.hello(["torrents","subscribe"])
    declarations=s.writes.map(function(w){return JSON.parse(w)}).filter(function(f){return f.op==="app.client"})
    check("dashboard_presence",declarations[0].args.dashboard_visible===false,"every connection declares its dashboard visibility",JSON.stringify(declarations))
    s=session();s.c.visibility(false,false,[],true);s.hello(["torrents","subscribe","dashboard_presence"])
    declarations=s.writes.map(function(w){return JSON.parse(w)}).filter(function(f){return f.op==="app.client"})
    check("dashboard_presence",declarations[0].args.overlay_visible===true && declarations[0].args.interactive===true && declarations[0].args.dashboard_visible===false,"palette declares actual visibility independently of dashboard",declarations)
    s.c.visibility(false,false,[],false)
    declarations=s.writes.map(function(w){return JSON.parse(w)}).filter(function(f){return f.op==="app.client"})
    check("dashboard_presence",declarations.slice(-1)[0].args.overlay_visible===false && declarations.slice(-1)[0].args.interactive===false,"closing palette clears shared presence",declarations)
    console.log("FBTQS-PASS dashboard_presence")
    Qt.callLater(function() { Quickshell.execDetached(["qs", "kill", "-p", Quickshell.shellPath(".")]) })
  }
}
