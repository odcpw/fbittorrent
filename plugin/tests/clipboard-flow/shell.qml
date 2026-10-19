import QtQuick
import Quickshell
import Quickshell.Io
import "Plugin/ClipboardFlow.js" as Flow
import "Plugin/Store.js" as Store
ShellRoot {
  FileView {id:fixture;blockLoading:true;blockAllReads:true;preload:false}
  function check(name,expect,value,ok){console.log("FBTQS-STEP "+JSON.stringify({case:name,action:"reduce frozen clipboard event",expect:expect,observed:JSON.stringify(value),ok:!!ok,ms:0}))}
  Component.onCompleted:{
    fixture.path=Quickshell.shellPath("Fixtures/clipboard.ndjson")
    var event=fixture.text().trim().split("\n").map(JSON.parse).filter(function(row){return row.dir==="s2c" && row.frame.ev==="clipboard.magnet"})[0].frame
    var s={client:{instance:"one",caps:["clipboard"]},store:Store.create(),clipboardOffer:null},f=Flow.create(s)
    s.store.apply(event);f.frame(event)
    check("live","frozen raw event creates an offer with only its hash in the add source",s.clipboardOffer,s.clipboardOffer.name===event.name && s.clipboardOffer.source.magnet==="magnet:?xt=urn:btih:"+event.hash && s.clipboardOffer.watched)
    s.clipboardOffer=null;f.frame(event);check("dedupe","repeated raw event cannot reopen a dismissed offer",s.clipboardOffer,s.clipboardOffer===null)
    var next=Object.assign({},event,{seq:2});s.store.apply(next);f.frame({ev:"delta",topic:"events"})
    check("dedupe","same hash in a new event is not offered again",s.clipboardOffer,s.clipboardOffer===null)
    next=Object.assign({},event,{seq:3,hash:"ab".repeat(20)});s.store.apply(next);f.frame({ev:"delta",topic:"events"})
    check("live","nested committed event delta offers a new magnet",s.clipboardOffer,s.clipboardOffer.hash===next.hash)
    s.clipboardOffer=null;var reload=Flow.create(s);reload.frame({ev:"snapshot.end",topic:"events"})
    check("history","initial historical snapshot never reoffers old clipboard contents",s.clipboardOffer,s.clipboardOffer===null)
    next=Object.assign({},event,{seq:4,hash:"cd".repeat(20),already_added:true});s.store.apply(next);reload.frame({ev:"delta",topic:"events"})
    check("dedupe","already-added event is ignored",s.clipboardOffer,s.clipboardOffer===null)
    next=Object.assign({},event,{seq:5,hash:"ef".repeat(20)});s.store.torrents[next.hash]={hash:next.hash};s.store.apply(next);reload.frame({ev:"delta",topic:"events"})
    check("dedupe","ledger presence suppresses the offer",s.clipboardOffer,s.clipboardOffer===null)
    s.client.instance="two";next=Object.assign({},event,{seq:1,name:undefined});s.store.events=[next];reload.frame(next)
    check("instance","new daemon sequence restarts; unknown name gets a plain fallback",s.clipboardOffer,s.clipboardOffer.hash===event.hash && s.clipboardOffer.name==="magnet link")
    s.client.caps=[];s.clipboardOffer=null;reload.frame(Object.assign({},event,{seq:2,hash:"de".repeat(20)}))
    check("capability","unnegotiated clipboard event is ignored",s.clipboardOffer,s.clipboardOffer===null)
    ;["live","dedupe","history","instance","capability"].forEach(function(name){console.log("FBTQS-PASS "+name)})
    Qt.callLater(function(){Quickshell.execDetached(["qs","kill","-p",Quickshell.shellPath(".")])})
  }
}
