import QtQuick
import Quickshell
import Quickshell.Io
import "Plugin/SearchFlow.js" as SearchFlow
import "Plugin/Store.js" as Store
ShellRoot {
  FileView {id:transcript;blockLoading:true;blockAllReads:true;preload:false}
  function check(name,condition,expect,observed){console.log("FBTQS-STEP "+JSON.stringify({case:name,action:"exercise production search controller",expect:expect,observed:JSON.stringify(observed),ok:!!condition,ms:0}))}
  function service(){
    var s={online:true,store:Store.create(),calls:[],searchTopics:[],selected:"",adds:[],copies:[]}
    s.client={caps:["search","subscribe","imdb","shows"],request:function(op,args,options){s.calls.push({op:op,args:args,done:options ? options.callback : null});return s.calls.length}}
    s.mutate=function(op,args,cap,done){s.calls.push({op:op,args:args,done:done});return "unit-"+s.calls.length}
    s.reply=function(op,result,failure){var row=s.calls.find(function(c){return c.op===op && c.done && !c.replied});if(!row)throw Error("no pending "+op);row.replied=true;row.done(result,failure)}
    s.scheduleSearchPoster=function(){};s.lookupImdb=function(args,done){s.calls.push({op:"imdb.lookup",args:args,done:done})}
    s.finishAdd=function(hash){s.selected=hash};s.beginSearchAdd=function(row,id){s.adds.push({row:row,id:id});return true}
    s.copyText=function(text){s.copies.push(text)};s.openUrl=function(url){s.opened=url};s.notify=function(text){s.note=text}
    return s
  }
  Component.onCompleted:{
    transcript.path=Quickshell.env("FBT_TEST_SEARCH_TRANSCRIPT") || Quickshell.shellPath("Fixtures/search.ndjson")
    var replay=Store.create(),lines=transcript.text().trim().split("\n").map(JSON.parse)
    lines.forEach(function(line){if(line.dir==="c2s")replay.noteRequest(line.frame);else if(line.dir==="s2c")replay.apply(line.frame)})
    var golden=replay.topics["search:1"]["a".repeat(40)]
    check("frozen_transcript",golden.added==="a".repeat(40) && golden.providers.join(",")==="alpha,beta","frozen search snapshot/delta uses key identity, retains deduplicated providers and applies the added marker",golden)
    console.log("FBTQS-PASS frozen_transcript")
    var s=service(),f=SearchFlow.create(s,function(){});f.mount();f.start("Sintel")
    s.reply("search.start",{search_id:7})
    check("session",s.searchTopics.join(",")==="search:7","search owns exactly its keyed result topic",s.searchTopics)
    var rows=[{key:"a",name:"Ten",size:10,seeds:10,uploaded_at:9,rating:9,providers:["alpha","beta"]},{key:"b",name:"Two",size:2,seeds:2,uploaded_at:10,rating:10,providers:["alpha"]},{key:"c",name:"Unknown",providers:["beta"]}]
    s.store.topics['search:7']={a:rows[0],b:rows[1],c:rows[2]};f.frame({ev:"snapshot.end",topic:"search:7"})
    check("typed_sort",f.rows[0].key==="a" && f.rows[2].key==="c","seeds sort numerically; unknown values remain last",f.rows)
    f.cycleSort();check("typed_sort",f.sort==="size" && f.rows[0].key==="a","size uses numeric10 before2",f.rows)
    f.cycleSort();check("typed_sort",f.sort==="uploaded" && f.rows[0].key==="b","uploaded timestamps use numeric10 before9",f.rows)
    f.cycleSort();check("typed_sort",f.sort==="name" && f.rows[0].key==="a","name compares words instead of numeric sentinels",f.rows)
    f.cycleSort();check("typed_sort",f.sort==="rating" && f.rows[0].key==="b","star rating uses numeric10 before9",f.rows)
    f.reverseSort();check("typed_sort",f.rows[0].key==="a" && f.rows[2].key==="c","reverse flips known values without pulling unknown values first",f.rows)
    rows[2].imdb_id="tt1234001";f.choose("c");f.lookup();s.reply("imdb.lookup",{card:{id:"tt1234001",title:"Local Fixture",rating:8.2,poster_path:"/private/poster.jpg"}})
    var count=s.calls.length;f.lookup();check("poster_cache",f.card.id==="tt1234001" && f.rating(rows[2])===8.2 && s.calls.length===count,"rest lookup supplies an unknown rating and reuses the IMDb-ID cache",f.card)
    rows[0].imdb_id="tt1234001";f.choose("a");f.lookup();check("poster_cache",f.card.id==="tt1234001" && s.calls.length===count && f.rating(rows[0])===9,"same IMDb ID reuses the card without overwriting a provider-supplied rating",f.card)
    f.choose("a");f.activate();check("grab",s.adds.length===1 && s.adds[0].id===7,"new result enters the shared review path with its session/key",s.adds)
    rows[0].added="a".repeat(40);f.activate();check("grab",s.selected==="a".repeat(40) && s.adds.length===1,"added result selects the existing ledger hash without another grab",s.selected)
    f.frame({topic:"events",ev:"delta",added:[{ev:"search.done",search_id:7,warnings:[]}]});check("session",f.phase==="done","nested search.done ends the searching state",f.snapshot())
    f.providersOpen();s.reply("search.providers",{providers:[{name:"alpha",kind:"nova",enabled:true}]});f.providerToggle(f.providers[0]);check("providers",f.providerBusy.alpha && s.calls.slice(-1)[0].args.enabled===false,"provider switch waits for confirmed server state",s.calls.slice(-1))
    s.reply("search.set_provider",{state:"done"});s.reply("search.providers",{providers:[{name:"alpha",kind:"nova",enabled:false}]});check("providers",!f.providerBusy.alpha && !f.providers[0].enabled,"provider acknowledgement refreshes the real enabled state",f.providers)
    f.browseOpen();s.reply("search.browse_sources",{sources:[{id:"3",label:"Latest movies",provider:"YTS",in_app:true}]});f.browse(f.sources[0]);s.reply("search.browse",{items:rows})
    check("browse",f.label==="Latest movies" && f.query==="Latest movies" && f.mode==="browse" && !s.searchTopics.length,"browse rows replace the old query label and release the search session",f.snapshot())
    var many=[];for(var i=0;i<700;i++)many.push({key:String(i),name:String(i),size:i,providers:["alpha"]})
    f.browse(f.sources[0]);s.reply("search.browse",{items:many});check("row_limit",f.rows.length===500 && f.total===700,"no more than500 search rows are rendered",f.snapshot().total)
    f.sort="size";f.reverse=false;f.resort();check("row_limit",f.rows[0].size===699,"browse sorting retains the hidden results before taking the500-row display window",f.rows[0])
    f.start("old");f.hide();s.reply("search.start",{search_id:9});check("session",s.calls.slice(-1)[0].op==="search.stop" && s.calls.slice(-1)[0].args.search_id===9 && !s.searchTopics.length,"late start reply after hide releases its newly allocated daemon session",s.calls.slice(-1))
    ;["session","typed_sort","grab","providers","browse","row_limit","poster_cache"].forEach(function(name){console.log("FBTQS-PASS "+name)})
    Qt.callLater(function(){Quickshell.execDetached(["qs","kill","-p",Quickshell.shellPath(".")])})
  }
}
