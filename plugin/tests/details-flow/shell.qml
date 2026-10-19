import QtQuick
import Quickshell
import "Plugin/DetailFlow.js" as DetailFlow
import "Plugin/Store.js" as Store
import "Plugin/DetailText.js" as DetailText
ShellRoot {
  function check(name,condition,expect,observed){console.log("FBTQS-STEP "+JSON.stringify({case:name,action:"exercise details controller and reducers",expect:expect,observed:JSON.stringify(observed),ok:!!condition,ms:0}))}
  function service(){
    var s={store:Store.create(),calls:[],topics:[],consumers:{}}
    s.client={caps:["files","subscribe","imdb","log"],request:function(op,args,options){s.calls.push({op:op,args:args,callback:options ? options.callback : null});return s.calls.length}}
    s.unsubscribeDetail=function(){s.store.releaseTopics(s.topics);s.topics=[]};s.subscribeDetail=function(hash,topics){var old=s.topics;s.topics=topics.map(function(t){return "torrent:"+hash+"."+t});s.store.releaseTopics(old.filter(function(t){return s.topics.indexOf(t)<0}))}
    s.logConsumer=function(id,on){s.consumers[id]=on};s.lookupImdb=function(args,done){return s.client.request("imdb.lookup",args,{callback:done})}
    s.setFiles=function(hash,ids,on,done){s.calls.push({op:"torrents.set_files",hash:hash,ids:ids,on:on,callback:done});return true}
    s.openTarget=function(hash,index){s.opened={hash:hash,index:index}}
    return s
  }
  Component.onCompleted:{
    var explanation=DetailText.networking({private:true,size:1024})
    check("private_networking",explanation.indexOf("Private torrent")>=0 && explanation.indexOf("no DHT")>=0 && explanation.indexOf("no PeX")>=0,"private metainfo explains disabled discovery",explanation)
    check("private_networking",DetailText.networking({private:false,size:1024})==="Public torrent" && !DetailText.networking({private:true}) && !DetailText.networking({size:1024}),"public and metadata-unknown rows do not claim disabled discovery",{})
    console.log("FBTQS-PASS private_networking")
    var s=service(),h="a".repeat(40),other="b".repeat(40),d=DetailFlow.create(s,function(){})
    s.store.apply({id:100,ok:true,result:{torrents:[{hash:h,name:"Local Fixture",state:"paused"},{hash:other,name:"Other",state:"paused"}]}},{op:"torrents.list"})
    function update(hash,patch){s.store.apply({id:101,ok:true,result:{torrent:Object.assign({},s.store.torrents[hash],patch)}},{op:"torrents.get",args:{hash:hash}})}
    d.begin(h);d.lookup();s.calls[0].callback({torrent:{hash:h,private:true,file_count:10}},null);s.calls[1].callback({card:null},null)
    d.hide();d.begin(h);d.lookup()
    check("lookup_cache",s.calls.length===2 && d.card===null,"get extras once per torrent and cache a null IMDb result",s.calls)
    d.selectTab("files")
    var rows={};[10,2,9,1,4,3,8,5,7,6].forEach(function(n,index){rows[index]={index:index,path:"E"+n+".mkv",sort_key:String(n).padStart(10,"0"),size:20,progress:0,included:true}})
    s.store.topics[s.topics[0]]=rows;d.frame({ev:"snapshot.end",topic:s.topics[0]})
    check("natural_files",d.files.map(function(f){return f.path}).join(",")==="E1.mkv,E2.mkv,E3.mkv,E4.mkv,E5.mkv,E6.mkv,E7.mkv,E8.mkv,E9.mkv,E10.mkv","plain sort_key order differs from file index order",d.files)
    d.toggle();check("file_actions",s.calls[2].ids[0]===3 && !s.calls[2].on,"Space skips the displayed file's true index",s.calls[2])
    d.filter("e10");d.setFiles(true,true);check("file_actions",s.calls[3].ids.join(",")==="0","all acts only on the case-insensitive filtered set",s.calls[3])
    d.filter("absent");d.setFiles(false,true);check("file_actions",s.calls[4].ids.length===0,"empty filtered set is sent without inventing an index",s.calls[4])
    update(h,{state:"checking"});var n=s.calls.length;d.toggle();d.setFiles(true,true);check("file_actions",s.calls.length===n,"checking refuses all file selection writes",s.calls.length)
    d.selectTab("log");check("topic_lifetime",!s.topics.length && s.consumers.details,"Log releases file sampling and acquires one log consumer",s)
    d.hide();check("topic_lifetime",!s.topics.length && !s.consumers.details,"hide releases the detail topics and log consumer",s.topics)
    d.begin(h);n=s.calls.length;d.frame({ev:"delta",topic:"events",added:[{ev:"engine.changed"}]});check("lookup_cache",s.calls.length===n+1,"nested engine.changed refetches detail extras",s.calls)
    s.client.caps.push("pieces");update(h,{state:"downloading",checked:true})
    var topic="torrent:"+h+".pieces",bits={count:8,piece_size:16384,have:"gA==",priority:[0,7]}
    function receivePieces(){s.store.setPieces(topic,bits);d.frame({ev:"state",topic:topic,data:bits})}
    d.selectTab("pieces");receivePieces();d.selectTab("overview")
    check("pieces_cache",!s.store.topics[topic],"hidden detail topics are released, not retained for every visited torrent",s.store.topics)
    d.selectTab("pieces")
    check("pieces_cache",s.store.topics[topic]===bits,"returning to the same checked torrent paints its last verified sample immediately",s.store.topics[topic])
    update(h,{checked:false});d.frame({ev:"delta",topic:"torrents"});d.selectTab("overview");d.selectTab("pieces")
    check("pieces_cache",!s.store.topics[topic],"checking invalidates the cache before reopening Pieces",s.store.topics)
    update(h,{checked:true});receivePieces();d.selectTab("overview");d.frame({ev:"delta",topic:"events",added:[{ev:"engine.changed"}]});d.selectTab("pieces")
    check("pieces_cache",!s.store.topics[topic],"an engine change invalidates even while another detail tab is visible",s.store.topics)
    receivePieces();d.selectTab("overview");update(h,{state:"paused"});d.frame({ev:"delta",topic:"torrents"});d.selectTab("pieces")
    check("pieces_cache",!s.store.topics[topic],"a torrent state change invalidates its prior live sample",s.store.topics)
    receivePieces();d.selectTab("overview");d.frame({id:10,ok:true,result:{}},{op:"torrents.set_files",ref:"ui-10"});d.selectTab("pieces")
    check("pieces_cache",!s.store.topics[topic],"an acknowledged mutation invalidates cached priorities",s.store.topics)
    receivePieces();d.selectTab("overview");d.frame({id:11,ok:false,error:{code:"busy"}},{op:"torrents.set_files",ref:"ui-11"});d.selectTab("pieces")
    check("pieces_cache",s.store.topics[topic]===bits,"a rejected mutation does not discard an unchanged verified sample",s.store.topics[topic])
    update(other,{checked:true});d.begin(other)
    check("pieces_cache",!s.store.topics[topic] && !s.store.topics["torrent:"+other+".pieces"],"changing the selected torrent cannot reuse another torrent's bits",s.store.topics)
    d.begin(h)
    receivePieces();d.hide();d.begin(h);d.selectTab("pieces")
    check("pieces_cache",!s.store.topics[topic],"hiding the Details pane discards its single cache",s.store.topics)
    console.log("FBTQS-PASS pieces_cache")
    var store=Store.create(),entries=[];for(var i=0;i<1010;i++)entries.push({seq:i,ts:0,hash:i%2 ? h : other,message:"entry "+i,level:"info",source:"engine"})
    store.apply({id:1,ok:true,result:{entries:entries,next_seq:1010}},{op:"app.log",args:{}})
    check("log_ring",Object.keys(store.topics.log).length===1000 && store.logEntries(h).length===500 && store.logEntries(h)[0].seq===11,"log uses seq keys, bounds the ring to 1000 and filters by hash",store.logEntries(h).slice(0,2))
    store.apply({id:2,ok:true,result:{sub:4,topics:["log"]}},{op:"subscribe",args:{topics:["log"]}})
    store.apply({ev:"delta",sub:4,topic:"log",rev:1,added:[{seq:1010,hash:h,message:"next"}]})
    check("log_ring",Object.keys(store.topics.log).length===1000 && store.logEntries(h).slice(-1)[0].seq===1010,"append delta keeps prior same-hash entries and enforces the bound",Object.keys(store.topics.log).length)
    var rejected=false;try{store.apply({ev:"delta",sub:4,topic:"log",rev:2,changed:{1010:{message:"rewritten"}}})}catch(error){rejected=String(error).indexOf("append-only")>=0}
    check("log_ring",rejected && store.logEntries(h).slice(-1)[0].message==="next","changed log entries are rejected without rewriting the append-only ring",rejected)
    ;["lookup_cache","natural_files","file_actions","topic_lifetime","log_ring"].forEach(function(name){console.log("FBTQS-PASS "+name)})
    Qt.callLater(function(){Quickshell.execDetached(["qs","kill","-p",Quickshell.shellPath(".")])})
  }
}
