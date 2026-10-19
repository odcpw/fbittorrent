import QtQuick
import Quickshell
import "Plugin/ShowsFlow.js" as Shows
import "Plugin/AddFlow.js" as Add
import "Plugin/Store.js" as Store
ShellRoot {
 function assertThat(value,message){if(!value)throw Error(message)}
 Component.onCompleted:{
  var store=Store.create(),topic="shows.releases:inbox"
  store.apply({id:1,ok:true,result:{sub:1,topics:[topic]}},{op:"subscribe"})
  store.apply({ev:"snapshot.begin",sub:1,rev:1,topic:topic,count:2})
  store.apply({ev:"snapshot.chunk",sub:1,rev:2,topic:topic,items:[{show_imdb:"tt1",key:"S01E01"},{show_imdb:"tt2",key:"S01E01"}]})
  store.apply({ev:"snapshot.end",sub:1,rev:3,topic:topic})
  assertThat(Object.keys(store.topics[topic]).length===2,"the inbox keeps identical episode keys from different shows")
  store.apply({ev:"delta",sub:1,rev:4,topic:topic,removed:["tt1/S01E01"],changed:{"tt2/S01E01":{read:true}}})
  assertThat(!store.topics[topic]["tt1/S01E01"] && store.topics[topic]["tt2/S01E01"].read,"inbox removals and updates use show/episode identity")
  var calls=[],service={showsTopics:[],store:{topics:{shows:{tt1:{imdb:"tt1",title:"One",unread:1}},"shows.releases:inbox":{"tt1/S01E01":{show_imdb:"tt1",key:"S01E01",title:"Episode",read:false,transfer:"none"}}},mutations:{},settings:{}},client:{caps:["shows","rss","staging"],request:function(){return 1}},now:0,notes:[]}
  service.mutate=function(op,args,cap,done){calls.push({op:op,args:args,done:done});return "ref-"+calls.length}
  service.notify=function(message){service.notes.push(message)}
  service.addFlow=Add.create(service,function(){},false)
  var flow=Shows.create(service,function(){})
  flow.mount();assertThat(service.showsTopics[0]==="shows.releases:inbox" && flow.rows.length===1,"inbox mounts with one unread release")
  flow.pane(true);flow.primary();assertThat(calls[0].op==="shows.grab" && calls[0].args.stage===true,"Enter stages through the catalogue operation")
  calls[0].done({hash:"a".repeat(40)},null);service.addFlow.cancel();assertThat(calls[1].op==="torrents.discard" && !calls.some(function(c){return c.op==="shows.set_read"}),"Discard never marks a source read")
  calls[1].done({},null);flow.mark();assertThat(calls[2].op==="shows.set_read" && calls[2].args.read,"explicit read toggles independently of transfer")
  calls[2].done({},null);flow.undo();assertThat(calls[3].op==="shows.undo_read","undo requests the daemon-owned undo")
  calls[3].done({},null);flow.followOpen();flow.follow("tt1234567");assertThat(calls[4].args.imdb==="tt1234567" && calls[4].op==="shows.follow","follow uses an explicit IMDb id")
  calls[4].done({state:"pending",ref:"ref-5"},null);service.store.mutations["ref-5"]={status:"done",result:{show:{imdb:"tt1234567"}}};flow.frame({ev:"mutation.result"});assertThat(!flow.busy && !flow.panel,"pending actions complete through the daemon outcome")
  flow.select("tt1",false);assertThat(!service.store.topics["shows.releases:inbox"],"switching shows releases the previous release table")
  service.store.topics["shows.releases:tt1"]={episode:{key:"S01E01"}}
  flow.hide();assertThat(!service.showsTopics.length && !service.store.topics["shows.releases:tt1"] && !flow.rows.length,"hidden inbox releases observation and cached release rows")
  console.log("Shows controller: staged review, safe discard, read/undo, follow and observation passed")
  console.log("FBTQS-PASS shows_journey")
  Qt.callLater(function(){Quickshell.execDetached(["qs","kill","-p",Quickshell.shellPath(".")])})
 }
}
