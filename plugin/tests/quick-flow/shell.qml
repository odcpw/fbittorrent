import QtQuick
import Quickshell
import "QuickPlugin/QuickFlow.js" as QuickFlow
import "Plugin/AddSource.js" as Source
ShellRoot {
 function check(action,ok,value){console.log("FBTQS-STEP "+JSON.stringify({case:"quick_flow",action:action,expect:"bounded independent search on shared service; explicit adds and safe cancellation",observed:JSON.stringify(value),ok:!!ok,ms:0}))}
 Component.onCompleted:{
  var calls=[],notices=[],svc={online:true,quickVisible:false,quickTopics:[],parseAddSource:Source.parse,store:{topics:{},releaseTopics:function(){}},notify:function(text){notices.push(text)},mutate:function(op,args,cap,cb){calls.push({op:op,args:args,cb:cb});return "ref"}}
  var topics=[],topicChanges=0;Object.defineProperty(svc,"quickTopics",{get:function(){return topics},set:function(value){topics=value;topicChanges++}})
  var flow=QuickFlow.create(svc,function(){}),h="a".repeat(40)
  flow.open();check("open borrows the shared presence",svc.quickVisible,calls)
  flow.edited("s");flow.edited("so");flow.edited("some");check("typing does not churn subscriptions before debounce",calls.length===0 && topicChanges===0,topicChanges)
  check("magnet/path classification",!!flow.source("magnet:?xt=urn:btih:"+h) && !!flow.source("/tmp/a.torrent") && !flow.source("ordinary words") && !flow.source("/tmp/a\n.torrent"),calls)
  flow.search("First");flow.close();calls[0].cb({search_id:11},null)
  check("late search reply is retired after Esc",!svc.quickVisible && calls[1].op==="search.stop" && calls[1].args.search_id===11,calls)
  flow.open();flow.search("Second");calls[2].cb({search_id:12},null)
  var keyed={};for(var i=0;i<20;i++)keyed["k"+i]={key:"k"+i,name:"Result "+i}
  svc.store.topics["search:12"]=keyed;flow.frame({ev:"snapshot.end",topic:"search:12"});flow.move(100)
  check("first ten rows and bounded selection",flow.rows.length===10 && flow.current().key==="k9" && calls[2].args.limit===10,flow.snapshot())
  flow.edited("Third");check("editing retires old topic before another search",flow.rows.length===0 && svc.quickTopics.length===0 && calls[3].op==="search.stop",flow.snapshot())
  var n=calls.length;flow.search("magnet:?invalid");check("malformed source never becomes a search query",calls.length===n && flow.phase==="error",flow.snapshot())
  flow.search("https://tracker.invalid/?apikey=private");check("source URLs never become provider search queries",calls.length===n && flow.phase==="error",flow.snapshot())
  var closed=0;flow.add({path:"/tmp/local.torrent"},function(){closed++;flow.close()});flow.edited("ignored");check("pending add cannot turn into another request",flow.phase==="adding",flow.snapshot())
  calls[n].cb({state:"done"},null);check("explicit source adds directly without staging",closed===1 && calls[n].op==="torrents.add" && calls[n].args.stage===undefined && notices[0]==="Torrent added",calls[n])
  console.log("FBTQS-PASS quick_flow");Quickshell.execDetached(["qs","kill","-p",Quickshell.shellPath(".")])
 }
}
