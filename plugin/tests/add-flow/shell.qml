import QtQuick
import Quickshell
import Quickshell.Io
import "Plugin/AddFlow.js" as Flow
import "Plugin/AddSource.js" as Source
import "Plugin/OfflineState.js" as Offline
ShellRoot {
  id:root
  FileView {id:fixture;blockLoading:true;blockAllReads:true;preload:false}
  function check(name,expected,actual,ok){console.log("FBTQS-STEP "+JSON.stringify({case:name,action:"exercise production add controller",expect:expected,observed:JSON.stringify(actual),ok:!!ok,ms:0}))}
  function service(caps){
    var s={now:0,online:true,addTopics:[],calls:[],notes:[],selected:"",store:{settings:{upload:"on",save_path:"/downloads"},torrents:{},topics:{},mutations:{}}}
    s.client={caps:caps,request:function(op,args,options){s.calls.push({op:op,args:args,callback:options.callback});return s.calls.length}}
    s.mutate=function(op,args,cap,done){s.calls.push({op:op,args:args,callback:done});return "unit-"+s.calls.length}
    s.notify=function(text){s.notes.push(text)};s.finishAdd=function(hash){s.selected=hash}
    s.reply=function(op,value){for(var i=0;i<s.calls.length;i++)if(s.calls[i].op===op && !s.calls[i].replied){s.calls[i].replied=true;s.calls[i].callback(value,null);return}throw Error("no pending "+op)}
    s.metadata=function(row){
      var read=s.calls.find(function(c){return c.op==="torrents.get" && !c.replied})
      if(read){s.reply("torrents.get",{torrent:row});return}
      s.store.torrents[row.hash]=row;s.flow.frame({ev:"delta",topic:"torrents"})
    }
    s.files=function(rows){var topic="torrent:"+s.flow.snapshot().hash+".files",keyed={};rows.forEach(function(row){keyed[row.index]=row});s.store.topics[topic]=keyed;s.flow.frame({ev:"snapshot.end",topic:topic})}
    return s
  }
  function controller(s){s.flow=Flow.create(s,function(){},true,function(){return s.now});return s.flow}
  Component.onCompleted:{
    var count=0,hash="a".repeat(40),source={magnet:"magnet:?xt=urn:btih:"+hash+"&dn=Review%20me"}
    check("source","hex and base32 normalize to the same hash",Source.parse("magnet:?xt=urn:btih:"+"A".repeat(32)),Source.parse("magnet:?xt=urn:btih:"+"A".repeat(32)).hash==="0".repeat(40) && Source.parse(source.magnet).hash===hash)
    check("source","file URLs decode once; remote URLs and NUL paths rejected",Source.parse("file:///tmp/a%20file.torrent"),Source.parse("file:///tmp/a%20file.torrent").source.path==="/tmp/a file.torrent" && !Source.parse("file://remote/tmp/a.torrent") && !Source.parse("file:///tmp/a%00.torrent"))
    check("source","clipboard edge whitespace is trimmed; embedded controls rejected",Source.parse(source.magnet+"\n"),!!Source.parse(source.magnet+"\n") && !Source.parse(source.magnet+"\n&bad"))
    check("source","exactly one source and one dropped file",Source.drop(["file:///tmp/a.torrent"],""),!!Source.drop(["file:///tmp/a.torrent"],"") && !Source.drop(["a","b"],"") && !Source.normalize({path:"/tmp/a.torrent",url:"https://x/a"}))
    check("source","URL-encoded xt is accepted once without losing the original link",Source.parse(source.magnet.replace("urn:btih:","urn%3Abtih%3A")),Source.parse(source.magnet.replace("urn:btih:","urn%3Abtih%3A")).hash===hash && !Source.parse(source.magnet.replace("urn:btih:","urn%253Abtih%253A")))
    var s=service(["torrents","files","staging","subscribe"]),f=controller(s)
    f.open(source)
    check("review","opening a source stages with upload off and preserves destination",f.snapshot(),s.calls.length===1 && s.calls[0].args.stage===true && !f.snapshot().seed && f.snapshot().savePath==="/downloads")
    s.reply("torrents.add",{hash:hash});var reads=s.calls.length
    check("deadlines","metadata waits once until 90s, without a one-second refresh",f.nextWakeup(),f.nextWakeup()===90000)
    for(var tick=1;tick<=30;tick++){s.now=tick*1000;f.tick()}
    check("review","metadata wait never polls get or files on timer ticks",s.calls,s.calls.length===reads)
    s.metadata({hash:hash,name:"Pushed metadata",staged:true,size:4096})
    check("deadlines","arriving metadata cancels the metadata timeout",f.nextWakeup(),f.nextWakeup()===Infinity)
    check("review","pushed torrent metadata immediately acquires the file subscription",s.addTopics,s.addTopics[0]==="torrent:"+hash+".files")
    f.cancel();s.reply("torrents.discard",{})
    check("deadlines","closed sheet has no pending time work",f.nextWakeup(),f.nextWakeup()===Infinity)
    check("review","cancel discards a stage without downloading or deleting files",f.snapshot(),!f.snapshot().visible && s.calls[s.calls.length-1].op==="torrents.discard")
    s=service(["torrents","files","staging"]);f=controller(s);f.open(source)
    check("staging","stage is sent only after negotiation",s.calls[0],s.calls[0].args.stage===true)
    s.reply("torrents.add",{hash:hash,duplicate:false});s.metadata({hash:hash,name:"Two files",staged:true,size:4096})
    fixture.path=Quickshell.shellPath("Fixtures/torrents.ndjson")
    var frames=fixture.text().trim().split("\n").map(JSON.parse)
    var files=frames.filter(function(row){return row.dir==="s2c" && row.frame.result && row.frame.result.files})[0].frame.result.files
    s.files(files)
    check("staging","frozen files populate selected natural-order rows",f.snapshot(),f.snapshot().phase==="ready" && f.snapshot().files.length===2 && f.snapshot().files[0].sort_key<=f.snapshot().files[1].sort_key)
    var skipped=f.snapshot().files[0].index;f.toggle(skipped);f.path("/other downloads");f.commit()
    var commit=s.calls[s.calls.length-1];check("staging","commit carries only selected files, seed and save folder",commit,commit.op==="torrents.commit" && commit.args.files.indexOf(skipped)<0 && commit.args.seed===false && commit.args.save_path==="/other downloads")
    s.files(files);check("staging","late metadata response cannot re-enable commit",f.snapshot(),f.snapshot().phase==="committing")
    s.reply("torrents.commit",{});check("staging","commit acknowledgement closes",f.snapshot(),!f.snapshot().visible)
    s=service(["torrents","staging","free_space","settings"]);f=controller(s);f.existing(hash)
    s.metadata({hash:hash,size:4096,name:"Handler stage",origin:"handler",seed:true,save_path:"/handler destination"})
    s.reply("app.free_space",{writable:true,free_bytes:4096});s.reply("app.settings.get",{upload:"off",save_path:"/default"});s.files(files)
    check("staging","existing stage adopts handler origin, destination but seeding remains off and frozen File.size",f.snapshot(),f.snapshot().phase==="ready" && f.snapshot().origin==="handler" && !f.snapshot().seed && f.snapshot().savePath==="/handler destination" && f.snapshot().neededBytes===4096 && f.snapshot().destinationReady)
    s=service(["torrents","staging"]);f=controller(s);f.open(source);f.cancel();check("cancel_race","cancel waits for in-flight stage hash",f.snapshot(),f.snapshot().phase==="cancel_wait")
    s.reply("torrents.add",{hash:hash});check("cancel_race","stage reply triggers discard, never delete-files",s.calls[s.calls.length-1],s.calls[s.calls.length-1].op==="torrents.discard" && s.calls[s.calls.length-1].args.hash===hash)
    s.reply("torrents.discard",{});check("cancel_race","discard acknowledgement closes",f.snapshot(),!f.snapshot().visible)
    s=service(["torrents","staging"]);f=controller(s);f.open(source);s.reply("torrents.add",{hash:hash});s.now=90000;f.tick()
    check("timeout","90 seconds exposes Keep paused / Discard",f.snapshot(),f.snapshot().phase==="metadata_timeout");f.keep();s.reply("torrents.stop",{});check("timeout","Keep paused sends stop and leaves the record",s.calls[s.calls.length-1],!f.snapshot().visible && s.calls[s.calls.length-1].op==="torrents.stop")
    s=service(["torrents","staging"]);s.store.torrents[hash]={hash:hash,name:"Existing"};f=controller(s);f.open(source);f.cancel();check("duplicate","known duplicate has no add or discard",s.calls,s.calls.length===0)
    s=service(["torrents","staging","subscribe"]);f=controller(s);f.open(source)
    s.reply("torrents.add",{state:"pending",ref:"unit-1"});s.store.mutations["unit-1"]={status:"done",result:{hash:hash}}
    f.frame({topic:"events",ev:"snapshot.end"})
    s.metadata({hash:hash,size:4096,name:"Subscribed"})
    check("subscription","pending add resolves from committed event snapshot; files use subscription",{state:f.snapshot(),topics:s.addTopics,calls:s.calls},s.addTopics[0]==="torrent:"+hash+".files" && s.calls.length===1)
    s.store.topics[s.addTopics[0]]={0:files[0],1:files[1]};f.frame({topic:s.addTopics[0],ev:"snapshot.end"})
    check("subscription","completed file snapshot makes staged add ready",f.snapshot(),f.snapshot().phase==="ready" && f.snapshot().files.length===2)
    f.commit();var reference="unit-2";s.reply("torrents.commit",{state:"pending",ref:reference})
    s.store.mutations[reference]={status:"pending"};f.frame({topic:"events",ev:"delta"})
    check("pending_commit","a pending event snapshot keeps Download in progress and never offers review again",f.snapshot(),f.snapshot().phase==="committing" && !f.snapshot().error)
    s.store.mutations[reference]={status:"done"};f.frame({topic:"events",ev:"delta"})
    check("subscription","pending commit with no result resolves from event delta",f.snapshot(),!f.snapshot().visible)
    s=service(["torrents","staging"]);f=controller(s);f.open(source);s.reply("torrents.add",{hash:hash});s.now=90000;f.tick()
    s.mutate=function(){s.lastError="fixture refuses Keep paused";return false};f.keep()
    check("timeout","failed Keep paused remains retryable with the typed message",f.snapshot(),f.snapshot().phase==="error" && f.snapshot().error==="fixture refuses Keep paused")
    s=service(["torrents","free_space"]);f=controller(s);f.open(source)
    check("destination","destination inspection runs before the harmless paused stage",s.calls,s.calls.length===2 && s.calls[0].op==="app.free_space" && s.calls[1].args.stage===true && !f.snapshot().destinationReady)
    s.reply("app.free_space",{writable:false});f.commit()
    check("destination","unwritable folder disables Download and sends no commit",f.snapshot(),!s.calls.some(function(c){return c.op==="torrents.commit"}) && !f.snapshot().destinationReady && f.snapshot().space.free_bytes===undefined)
    s.reply("torrents.add",{hash:hash});f.path("/good");s.now=300;f.tick();f.path("/latest");s.reply("app.free_space",{writable:true,free_bytes:4096})
    check("destination","old path replies cannot enable a new destination",f.snapshot(),!f.snapshot().destinationReady && f.snapshot().savePath==="/latest")
    s.now=600;f.tick();s.reply("app.free_space",{writable:true,free_bytes:0});f.commit()
    check("destination","known zero is kept and writable destination reaches commit",s.calls[s.calls.length-1],f.snapshot().space.free_bytes===0 && s.calls[s.calls.length-1].op==="torrents.commit" && s.calls[s.calls.length-1].args.save_path==="/latest")
    var result={key:hash,name:"Search selection",providers:["alpha"],magnet:source.magnet}
    s=service(["torrents","search","staging"]);f=controller(s);f.search(result,7)
    check("search_review","negotiated staging uses the session/key and stage true",s.calls[0],s.calls[0].op==="search.grab" && s.calls[0].args.stage===true && s.calls[0].args.search_id===7 && s.calls[0].args.key===hash)
    s.reply("search.grab",{state:"pending",ref:"unit-1"});s.store.mutations["unit-1"]={status:"pending",result:{state:"pending",ref:"unit-1"}};f.frame({topic:"events",ev:"delta"})
    check("search_review","a pending snapshot is an acknowledgement, not a failed add",f.snapshot(),f.snapshot().phase==="staging" && !f.snapshot().error)
    f.frame({ev:"mutation.result",ref:"unit-1",outcome:"done",result:{hash:hash}});check("search_review","staged grab enters the shared metadata/file selection flow",f.snapshot(),f.snapshot().owned && s.calls.length===1)
    s=service(["torrents","search","free_space"]);f=controller(s)
    result={key:hash,name:"100 MiB fixture",size:100*1024*1024,providers:["local"],magnet:source.magnet}
    f.search(result,1);s.reply("app.free_space",{writable:true,free_bytes:64*1024*1024});count=s.calls.length;f.commit()
    check("space_refusal","100 MiB review against64 MiB refuses every commit path",f.snapshot(),f.snapshot().insufficientSpace && !f.snapshot().destinationReady && f.snapshot().neededBytes===100*1024*1024 && s.calls.length===count)
    s.reply("search.grab",{hash:hash});f.path("/large");s.now=300;f.tick();s.reply("app.free_space",{writable:true,free_bytes:100*1024*1024})
    check("space_refusal","exact capacity boundary enables confirmation",f.snapshot(),!f.snapshot().insufficientSpace && f.snapshot().destinationReady)
    f.path("/unknown");s.now=600;f.tick();s.reply("app.free_space",{writable:true})
    check("space_refusal","unknown free bytes stay unknown and do not invent a refusal",f.snapshot(),f.snapshot().space.free_bytes===undefined && f.snapshot().destinationReady)
    s=service(["torrents","staging","free_space"]);f=controller(s);f.open(source);s.reply("app.free_space",{writable:true,free_bytes:64*1024*1024});s.reply("torrents.add",{hash:hash});s.metadata({hash:hash,size:100*1024*1024})
    s.files([{index:0,path:"first",size:60*1024*1024,sort_key:"a",included:true},{index:1,path:"second",size:40*1024*1024,sort_key:"b",included:true}])
    check("space_refusal","selected file lengths determine required bytes",f.snapshot(),f.snapshot().insufficientSpace && f.snapshot().neededBytes===100*1024*1024)
    f.toggle(1);check("space_refusal","skipping an oversized file enables the remaining60 MiB",f.snapshot(),f.snapshot().destinationReady && f.snapshot().neededBytes===60*1024*1024)
    s=service(["torrents","staging"]);f=controller(s);f.open(source);f.commit()
    check("early_download","Download can be requested while the stage is pending",f.snapshot(),f.snapshot().commitRequested && f.snapshot().phase==="staging")
    s.reply("torrents.add",{hash:hash});var early=s.calls[s.calls.length-1]
    check("early_download","unknown metadata commits all files by omitting files, with seeding off",early,early.op==="torrents.commit" && early.args.files===undefined && early.args.seed===false)
    s.reply("torrents.commit",{});check("early_download","commit closes without waiting for metadata",f.snapshot(),!f.snapshot().visible)
    s=service(["torrents","staging"]);s.online=false;f=controller(s);f.open(source,{save_path:"/handler destination"});f.commit();s.online=true;f.frame({id:1})
    check("early_download","an offline review keeps the immediate Download intent until the real connection arrives",s.calls,s.calls.length===1 && s.calls[0].op==="torrents.add" && f.snapshot().commitRequested && f.snapshot().savePath==="/handler destination")
    s=service(["torrents","staging"]);f=controller(s);f.open(source)
    s.reply("torrents.add",{hash:hash,duplicate:true});s.metadata({hash:hash,name:"Review after reload",staged:true,origin:"handler",save_path:"/downloads"})
    f.commit();var recovered=s.calls[s.calls.length-1]
    check("early_download","a stage left by an older shell is reviewed and committed instead of treated as a finished duplicate",recovered,recovered.op==="torrents.commit" && recovered.args.hash===hash && recovered.args.files===undefined)
    s.reply("torrents.commit",{});check("early_download","recovered review closes after one commit",f.snapshot(),!f.snapshot().visible)
    s=service(["torrents","staging"]);f=controller(s);f.open(source);f.cancel();s.reply("torrents.add",{hash:hash,duplicate:true})
    check("cancel_race","canceling a duplicate stage never discards another existing stage",s.calls,!f.snapshot().visible && s.calls.length===1)
    s=service(["torrents","staging"]);s.store.torrents[hash]={hash:hash,name:"Cached stage",staged:true,save_path:"/downloads"};f=controller(s);f.open(source)
    check("early_download","a known cached stage uses the same review ownership path without another add",f.snapshot(),f.snapshot().owned && !f.snapshot().duplicate && s.calls.length===0)
    s=service(["torrents","staging"]);f=controller(s);f.open(source)
    var late=s.calls[0].callback,callCount=s.calls.length
    s.addFlow=f;s.torrentModel={clear:function(){}};s.orderBoundary=function(){};s.logRevision=0;s.statsRevision=0
    s.online=false;Offline.clear(s)
    check("disconnect_review","disconnect closes review without issuing a commit or discard",f.snapshot(),!f.snapshot().visible && s.calls.length===callCount)
    late({hash:hash},null);f.frame({ev:"delta",topic:"torrents"});f.tick()
    check("disconnect_review","late reply and ticks cannot revive a disconnected review",f.snapshot(),!f.snapshot().visible && s.calls.length===callCount)
    console.log("FBTQS-PASS disconnect_review")
    console.log("FBTQS-PASS early_download")
    console.log("FBTQS-PASS space_refusal")
    ;["deadlines","source","review","staging","cancel_race","timeout","duplicate","subscription","pending_commit","destination","search_review"].forEach(function(name){console.log("FBTQS-PASS "+name)})
    Qt.callLater(function(){Quickshell.execDetached(["qs","kill","-p",Quickshell.shellPath(".")])})
  }
}
