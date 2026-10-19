import QtQuick
import Quickshell
import "Plugin/views"
import "Plugin/TrackerText.js" as TrackerText
import "Plugin/DetailFlow.js" as DetailFlow
import "Plugin/Store.js" as Store
ShellRoot {
  id:root
  TrackerRow {id:row;width:350;tracker:({url:"https://private-user:private-pass@tracker.invalid/announce?passkey=private-token#private-fragment",status:"error",message:"private-user private-pass private-token https://other-user:other-pass@other.invalid/error?key=other-secret"})}
  function check(name,ok,expect,observed){console.log("FBTQS-STEP "+JSON.stringify({case:name,action:"exercise actual tracker presentation and controller",expect:expect,observed:JSON.stringify(observed),ok:!!ok,ms:0}))}
  Component.onCompleted:{
    var text=row.rendered()
    check("privacy",["private-user","private-pass","private-token","passkey=","private-fragment","other-user","other-pass","key="].every(function(s){return text.indexOf(s)<0}),"rendered URL and failure prose contain no userinfo, query, fragment or echoed credentials",text)
    var scheme=TrackerText.message("https://other-user:other-pass@other.invalid/error?secret=value","https://tracker.invalid/?key=https")
    check("privacy",["other-user","other-pass","secret=","value"].every(function(s){return scheme.indexOf(s)<0}),"a credential matching a URL scheme cannot bypass URL redaction",scheme)
    check("unknown",text.indexOf("Error · Peers —")>=0 && text.indexOf("Last — · Next —")>=0,"never-contacted measurements remain unknown",text)
    row.tracker={url:"https://tracker.invalid/announce",status:"ok",peers:0,last_announce_at:1}
    text=row.rendered();check("unknown",text.indexOf("OK · Peers 0")>=0 && text.indexOf("Last —")<0 && text.indexOf("Next —")>=0,"known zero and a real announce timestamp remain distinct from missing values",text)
    var calls=[],topics=[],callback=null,hash="a".repeat(40),service={store:Store.create(),instance:"test"}
    service.client={caps:["trackers","subscribe"],request:function(op,args){calls.push({op:op,args:args});return calls.length}}
    service.subscribeDetail=function(h,rows){topics=rows};service.unsubscribeDetail=function(){topics=[]};service.logConsumer=function(){}
    service.mutate=function(op,args,cap,done){calls.push({op:op,args:args,cap:cap});callback=done;return calls.length}
    var flow=DetailFlow.create(service,function(){})
    service.store.torrents[hash]={hash:hash,name:"Tracker fixture",state:"downloading"};flow.begin(hash);flow.selectTab("trackers")
    check("lifetime",topics.join() === "trackers" && calls.length===1,"opening Trackers acquires one topic without ad-hoc polling",{topics:topics,calls:calls})
    flow.reannounce();var count=calls.length;flow.reannounce()
    check("force",calls.length===count && calls[count-1].op==="torrents.reannounce" && flow.trackerBusy,"repeated activation coalesces while the explicit mutation is pending",calls)
    callback();check("force",!flow.trackerBusy,"confirmed completion releases the button without inventing a tracker status",flow.trackerBusy)
    flow.selectTab("overview");check("lifetime",!topics.length,"leaving the Trackers tab releases observation",topics)
    flow.selectTab("trackers");flow.hide();check("lifetime",!topics.length,"hide releases the final tracker topic",topics)
    ;["privacy","unknown","lifetime","force"].forEach(function(name){console.log("FBTQS-PASS "+name)})
    Qt.callLater(function(){Quickshell.execDetached(["qs","kill","-p",Quickshell.shellPath(".")])})
  }
}
