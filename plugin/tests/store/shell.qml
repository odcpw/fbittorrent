import QtQuick
import Quickshell
import Quickshell.Io
import "Plugin/Store.js" as Store

ShellRoot {
  id: root
  property int resets: 0
  property int updates: 0
  ListModel { id: mirror; dynamicRoles: true }
  Connections {
    target: mirror
    function onModelReset() { root.resets++ }
    function onDataChanged() { root.updates++ }
  }
  FileView { id: input; blockLoading: true; blockAllReads: true; preload: false }
  function canonical(value) {
    if (Array.isArray(value)) return "[" + value.map(canonical).join(",") + "]"
    if (value !== null && typeof value === "object") return "{" + Object.keys(value).sort().map(function(k) {return JSON.stringify(k)+":"+canonical(value[k])}).join(",") + "}"
    return JSON.stringify(value)
  }
  function check(name, action, expected, observed, ok) {
    console.log("FBTQS-STEP " + JSON.stringify({case:name, action:action, expect:expected,
      observed:String(observed), ok:!!ok, ms:0, artifacts:[]}))
  }
  function corpus() {
    var names = JSON.parse(Quickshell.env("FBT_FIXTURE_NAMES"))
    var metadata = JSON.parse(Quickshell.env("FBT_FIXTURE_METADATA"))
    var required=["hello.ndjson","errors.ndjson","torrents.ndjson","subscribe.ndjson","subscribe-gap.ndjson","mutations.ndjson","imdb.ndjson"], totalExpected=0
    check("coverage","load frozen groups","every required transcript group is present",JSON.stringify(names),required.every(function(name){return names.indexOf(name)>=0}))
    names.forEach(function(name) {
      var store = Store.create(), count = 0, expectedCount = 0
      input.path = Quickshell.shellPath("Fixtures/" + name)
      var content = input.text(), lines = content.trim().split("\n")
      check(name.replace(".ndjson","").replace(/-/g,"_"), "read exact shared fixture bytes", "digest and line count match the source file", Qt.md5(content) + "; lines=" + lines.length,
            Qt.md5(content) === metadata[name].md5 && lines.length === metadata[name].lines)
      lines.forEach(function(line, index) {
        var record = JSON.parse(line), ok = true, observed = "applied"
        try {
          if (record.dir === "c2s") store.noteRequest(record.frame)
          else if (record.dir === "s2c") { store.apply(record.frame); count++ }
          else if (record.dir === "expect") {
            expectedCount++; totalExpected++
            var model = store.model(Object.keys(record.model))
            ok = canonical(model) === canonical(record.model)
            observed = "expected=" + canonical(record.model) + "; observed=" + canonical(model)
          }
        } catch (error) { ok=false; observed=String(error) }
        check(name.replace(".ndjson","").replace(/-/g,"_"), "apply " + name + ":" + (index+1) + " " + record.dir,
              record.dir === "expect" ? "store exactly matches frozen expected model" : "shared frame reduces without error", observed, ok)
      })
      if (name === "mutations.ndjson") {
        check("mutations","replay mutation results and event batch","refs correlate done, failed and timeout; replayed seq deduplicated",canonical(store.mutations),
          store.mutations["test-add"] && store.mutations["test-add"].status==="done" && store.mutations["test-remove"] && store.mutations["test-remove"].status==="failed" && store.mutations["test-timeout"] && store.mutations["test-timeout"].status==="timeout" &&
          store.events.filter(function(e){return e.seq===6}).length===1)
      }
      check(name.replace(".ndjson","").replace(/-/g,"_"), "finish shared transcript", "at least one server frame applied", "frames="+count+"; expect models="+expectedCount, count>0)
      console.log("FBTQS-PASS " + name.replace(".ndjson","").replace(/-/g,"_"))
    })
    check("coverage","compare every frozen expected model","at least the five original expect records compared","expect records="+totalExpected,totalExpected>=5)
    console.log("FBTQS-PASS coverage")
  }
  function benchmark() {
    var samples=[], rows=[]
    for(var i=0;i<500;i++) rows.push({hash:i.toString(16).padStart(40,"0"),name:"Torrent "+(500-i),state:"downloading",added_at:i,progress:i/500,dl_bps:500-i})
    for(var repeat=0;repeat<30;repeat++) {
      var store=Store.create()
      store.apply({id:1,ok:true,result:{torrents:rows}},{op:"torrents.list"})
      var began=Date.now(), result=store.sort("name",false), elapsed=Date.now()-began
      samples.push(elapsed)
      check("sort_500", "sort 500 rows, cold cache iteration "+repeat, "elapsed < 2 ms and 500 keys", "elapsed="+elapsed+" ms; keys="+result.length, elapsed<2 && result.length===500)
    }
    check("sort_500","report every timing sample","30 cold-cache sorts meet budget",JSON.stringify(samples),samples.every(function(t) {return t<2}))
    console.log("FBTQS-PASS sort_500")
  }
  function controls() {
    var hash="a".repeat(40), other="b".repeat(40), now=0, gaps=[]
    var store=Store.create({now:function(){return now},resubscribe:function(topic){gaps.push(topic)}})
    var row={hash:hash,name:"One",state:"downloading",intent:"run",progress:0.25,eta_s:60,added_at:2,dl_bps:1}
    store.apply({id:1,ok:true,result:{torrents:[row]}},{op:"torrents.list"})
    store.syncModel(mirror,"added",false)
    var original=store.torrents[hash], resetBefore=resets, updateBefore=updates
    store.apply({id:2,ok:true,result:{sub:1,topics:["torrents"]}},{op:"subscribe"})
    store.apply({ev:"delta",sub:1,rev:1,topic:"torrents",changed:{[hash]:{progress:0.5,eta_s:null}}})
    store.syncModel(mirror,"added",false)
    check("mirror","apply progress and deletion patch","stable row identity; changed Qt roles; no reset",JSON.stringify(mirror.get(0)),
      store.torrents[hash]===original && !Object.prototype.hasOwnProperty.call(original,"eta_s") &&
      mirror.get(0).eta_s===null && mirror.get(0).progress===0.5 && resets===resetBefore && updates>updateBefore)
    store.apply({ev:"delta",sub:1,rev:2,topic:"torrents",changed:{[hash]:{state:"paused"}}})
    store.syncModel(mirror,"added",false)
    check("mirror","change state","incremental counts and filter use confirmed state",JSON.stringify(store.counts),
      store.counts.downloading===0 && store.counts.paused===1 && store.filter("paused")[0]===hash && mirror.get(0).state==="paused")
    store.apply({ev:"delta",sub:1,rev:3,topic:"torrents",added:[{hash:other,name:"Two",state:"done",added_at:3}],removed:[hash]})
    store.syncModel(mirror,"added",false)
    check("mirror","add/remove membership","only member changes insert/remove; no reset",JSON.stringify(mirror.get(0)),mirror.count===1 && mirror.get(0).hash===other && resets===resetBefore)
    store.apply({id:3,ok:true,result:{state:"done",hash:hash}},{op:"torrents.add",ref:"pending-fixture"})
    check("mirror","complete mutation reply","mutation result never invents a torrent row",JSON.stringify(store.mutations),Object.keys(store.torrents).length===1 && !store.torrents[hash])
    console.log("FBTQS-PASS mirror")
    store.apply({ev:"delta",sub:1,rev:5,topic:"torrents",removed:[other]})
    check("gap_negative","skip a revision","suspect delta rejected; row retained; resubscribe raised",JSON.stringify(gaps),!!store.torrents[other] && gaps.length===1 && store.subscriptions[1].rev===3)
    console.log("FBTQS-PASS gap_negative")
    store=Store.create();store.apply({id:1,ok:true,result:{sub:2,topics:["torrents","summary"]}},{op:"subscribe"})
    store.apply({ev:"snapshot.begin",sub:2,rev:1,topic:"torrents",count:1})
    var rejected=false
    try {store.apply({ev:"state",sub:2,rev:2,topic:"summary",data:{}})} catch(error) {rejected=true}
    check("snapshot_negative","interleave a state frame","interleaved snapshot/state rejected",rejected,rejected)
    store=Store.create();store.apply({id:1,ok:true,result:{sub:1,topics:["torrents"]}},{op:"subscribe"})
    store.apply({ev:"snapshot.begin",sub:1,rev:1,topic:"torrents",count:1})
    rejected=false
    try {store.apply({ev:"snapshot.end",sub:1,rev:2,topic:"torrents"})} catch(error) {rejected=true}
    check("snapshot_negative","end an incomplete snapshot","missing snapshot item rejected",rejected,rejected)
    console.log("FBTQS-PASS snapshot_negative")
    store=Store.create({now:function(){return now}})
    store.apply({id:1,ok:true,result:{torrents:[{hash:hash,name:"z",state:"paused",dl_bps:1,added_at:2},
      {hash:other,name:"A",state:"downloading",dl_bps:2,progress:0.8,added_at:1}]}},{op:"torrents.list"})
    var names=store.sort("Name",false), rates=store.sort("↓",false)
    store.apply({id:2,ok:true,result:{sub:1,topics:["torrents"]}},{op:"subscribe"})
    store.apply({ev:"delta",sub:1,rev:1,topic:"torrents",changed:{[hash]:{dl_bps:9}}})
    check("typed_sort","change only rate","name order cached; rate sorting throttled for one second",names.join(","),
      names[0]===other && names===store.sort("Name",false) && rates===store.sort("↓",false))
    now=1000
    check("typed_sort","advance one second","numeric rate order updates and absent progress stays last",store.sort("↓",false).join(","),
      store.sort("↓",false)[0]===hash && store.sort("Progress",true)[1]===hash)
    console.log("FBTQS-PASS typed_sort")
    store=Store.create()
    var topic="torrent:"+hash+".peers", seed={addr:"127.0.0.1:1",conn:"tcp_mse",incoming:false,state:"live",downloaded:1,uploaded:0,seed:true}
    var leecher={addr:"127.0.0.1:2",conn:"tcp",incoming:true,state:"live",downloaded:0,uploaded:1,seed:false}
    store.apply({id:1,ok:true,result:{peers:[seed,leecher]}},{op:"torrents.peers",args:{hash:hash}})
    check("peer_details","read peers","two distinct addr identities survive the read",JSON.stringify(store.topics[topic]),Object.keys(store.topics[topic]).length===2)
    store.apply({id:2,ok:true,result:{sub:1,topics:[topic]}},{op:"subscribe"})
    store.apply({ev:"snapshot.begin",sub:1,topic:topic,rev:1,count:2,reason:"subscribe"})
    store.apply({ev:"snapshot.chunk",sub:1,topic:topic,rev:2,items:[seed,leecher]})
    store.apply({ev:"snapshot.end",sub:1,topic:topic,rev:3})
    var changes={};changes[seed.addr]={downloaded:16385,dl_bps:16384}
    store.apply({ev:"delta",sub:1,topic:topic,rev:4,changed:changes})
    check("peer_details","apply addr-keyed rates","seed rate changes without losing the other peer",JSON.stringify(store.topics[topic]),store.topics[topic][seed.addr].dl_bps===16384 && store.topics[topic][leecher.addr].seed===false)
    console.log("FBTQS-PASS peer_details")

  }
  Component.onCompleted: {
    corpus(); controls(); benchmark()
    Qt.callLater(function() { Quickshell.execDetached(["qs","kill","-p",Quickshell.shellPath(".")]) })
  }
}
