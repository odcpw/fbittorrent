import QtQuick
import Quickshell
import "Plugin"
import "Plugin/Store.js" as Store
import "Plugin/KeyedList.js" as KeyedList
ShellRoot {
 Service {id:svc}
 property int resets:0
 property int updates:0
 ListModel {id:mirror}
 Connections {target:mirror;function onModelReset(){resets++} function onDataChanged(){updates++}}
 function check(name,ok,expect,actual){console.log("FBTQS-STEP "+JSON.stringify({case:name,action:"repeat unrelated frames and append log entries",expect:expect,observed:JSON.stringify(actual),ok:ok,ms:0}));console.log("FBTQS-PASS "+name)}
 Component.onCompleted:Qt.callLater(function(){
  var client={online:false,reconnecting:false,instance:"",actions:{},state:function(){return {pending:[]}},request:function(){return 1},nextWakeup:function(){return Infinity},tick:function(){},mode:"stopped",caps:[],lastError:null}
  var before=svc.actionRevision
  for(var i=0;i<100;i++)svc.syncClient(client)
  var actions=svc.actionRevision-before
  client.actions.one={op:"torrents.recheck",args:{hashes:["a".repeat(40)]},status:"pending"};svc.syncClient(client)
  var revision=svc.actionRevision;svc.syncClient(client)
  check("action_transitions",svc.actionRevision===revision && svc.pendingAction("a".repeat(40)).op==="torrents.recheck","unchanged pending actions retain their revision and keyed lookup",svc.actionRevision)
  client.actions.one.status="confirmed";svc.syncClient(client)
  check("action_confirmation",svc.actionRevision===revision+1 && !svc.pendingAction("a".repeat(40)),"confirmation publishes exactly one removal",svc.actionRevision)
  client.lastError={code:"timeout",message:"old timeout"};svc.syncClient(client);svc.notify("old timeout · Refresh")
  client.lastError=null;svc.syncClient(client)
  check("error_success",!svc.lastError && !svc.lastMessageText && !svc.toast,"successful client recovery clears the stale footer and toast",{error:svc.lastError,message:svc.lastMessageText,toast:svc.toast})
  svc.notify("old timeout");client.instance="new-instance";svc.syncClient(client)
  check("error_restart",!svc.lastMessageText && !svc.toast,"instance reset clears stale toast and footer",svc.lastMessageText)
  var a="a".repeat(40),b="b".repeat(40)
  client.online=true;svc.client=client;svc.syncClient(client)
  svc.applyFrame({id:1,ok:true,result:{torrents:[{hash:a,name:"Selected",state:"downloading",size:4096,checked:true},{hash:b,name:"Other",state:"downloading"}]}},{op:"torrents.list"})
  svc.detailFlow.begin(a);svc.store.subscriptions[1]={rev:0,topics:["torrents"]}
  before=svc.detailRevision
  for(i=1;i<=100;i++){var changed={};changed[b]={dl_bps:i};svc.applyFrame({ev:"delta",sub:1,rev:i,topic:"torrents",changed:changed})}
  var details=svc.detailRevision-before
  var selected={};selected[a]={state:"paused"}
  svc.applyFrame({ev:"delta",sub:1,rev:101,topic:"torrents",changed:selected})
  check("selected_change",svc.detailRevision===before+1 && svc.detailFlow.row().state==="paused","selected torrent changes still update the cached detail row",svc.detailRevision-before)
  var store=Store.create(),entries=[]
  for(i=0;i<1000;i++)entries.push({seq:i,ts:0,source:"engine",level:"info",message:"entry "+i})
  store.apply({id:1,ok:true,result:{entries:entries}},{op:"app.log"})
  store.apply({id:2,ok:true,result:{sub:1,topics:["log"]}},{op:"subscribe",args:{topics:["log"]}})
  var at=Date.now()
  for(i=1000;i<2000;i++)store.apply({ev:"delta",sub:1,rev:i-999,topic:"log",added:[{seq:i,ts:0,source:"engine",level:"info",message:"entry "+i}]})
  var sorts=store.logSorts || 0
  var elapsed=Date.now()-at
  console.log("FBT-AUDIT-PERF "+JSON.stringify({actionRevisions:actions,detailRevisions:details,logSorts:sorts,logAppendMs:elapsed}))
  check("actions",actions===0,"100 unchanged ticks publish no action revisions",actions)
  check("details",details===0,"100 unrelated torrent changes publish no selected detail revision",details)
  check("logs",sorts===0 && store.logEntries("").length===1000 && store.logEntries("")[0].seq===1000,"1000 appends need no full-ring sort and retain the latest 1000",{sorts:sorts,ms:elapsed})
  var sync=KeyedList.create(),first={addr:"a",dl_bps:0},second={addr:"b",dl_bps:10}
  sync.sync(mirror,[first,second],"addr");var prior=resets
  sync.sync(mirror,[{addr:"a",dl_bps:200},second],"addr")
  check("keyed_update",resets===prior && updates>0 && mirror.get(0).entry.dl_bps===200 && mirror.count===2,"peer rate changes update native roles without resetting the list",{resets:resets,updates:updates})
  sync.sync(mirror,[second,{addr:"c",dl_bps:30}],"addr")
  check("keyed_membership",resets===prior && mirror.count===2 && mirror.get(0).entry.addr==="b" && mirror.get(1).entry.addr==="c","membership edits retain matching keys without a list reset",{resets:resets,count:mirror.count})
  var order=store.logEntries(""),sources=store.logSources(),version=store.logVersion
  store.apply({id:3,ok:true,result:{entries:order}},{op:"app.log"})
  check("log_dedupe",store.logVersion===version && store.logEntries("")===order && store.logSources()===sources,"replayed log rows retain their cached order and sources",store.logVersion)
  Qt.callLater(function(){Quickshell.execDetached(["qs","kill","-p",Quickshell.shellPath(".")])})
 })
}
