import QtQuick
import Quickshell
import Quickshell.Io
import "Plugin/SettingsFlow.js" as Flow
import "Plugin/SettingsSchema.js" as Schema
import "Plugin/Store.js" as Store
import "Plugin/HelpModel.js" as Help
import "Plugin/LogModel.js" as Log
import "Plugin/DetailText.js" as Details
import "Plugin/Model.js" as Model
import "Plugin/PopupState.js" as Popup
ShellRoot {
 FileView {id:fixture;blockLoading:true;blockAllReads:true;preload:false}
 function check(name,expect,value,ok){console.log("FBTQS-STEP "+JSON.stringify({case:name,action:"exercise settings/log/help model",expect:expect,observed:JSON.stringify(value),ok:!!ok,ms:0}))}
 function rejected(key,value){try{Schema.parse(Schema.field(key),value);return false}catch(error){return true}}
 Component.onCompleted:{
  var expected=["max_active_downloads","peer_encryption","network_interface","save_path","upload","autostart","idle_exit_secs","notify","timeout_secs","handler","open_command","clipboard_watch","reduce_motion","peer_port","show_quality","download_limit_bps","upload_limit_bps"].sort()
  check("coverage","current settings have controls; retired alternate limits have none",Schema.fields.map(function(f){return f.key}).sort(),JSON.stringify(expected)===JSON.stringify(Schema.fields.map(function(f){return f.key}).sort()))
  check("validation","upload is explicit off/on; arbitrary values are rejected",null,Schema.parse(Schema.field("upload"),"off")==="off" && Schema.parse(Schema.field("upload"),"on")==="on" && rejected("upload","unlimited"))
  check("validation","Details uses effective upload permission even without completion seeding",null,Details.policy({seed:false,upload_enabled:true})==="Upload: on" && Details.policy({seed:true,upload_enabled:false})==="Upload: off")
  check("validation","missing files is visible in dashboard and popup without a percent claim",null,Model.stateLabel({state:"error",reason:"missing_files"})==="Files missing" && Popup.meta({state:"error",reason:"missing_files"},false)==="Files missing")
  check("limits","K/M values use binary bytes, empty/zero clears",null,Schema.parse(Schema.field("download_limit_bps"),"16K")===16384 && Schema.parse(Schema.field("upload_limit_bps"),"4M")===4194304 && Schema.parse(Schema.field("upload_limit_bps"),"")===null && Schema.parse(Schema.field("upload_limit_bps"),"0")===null)
  check("limits","limits bounds reject subminimum,overflow and fractional syntax",null,rejected("upload_limit_bps","16383") && rejected("download_limit_bps","4096M") && rejected("download_limit_bps","1.5M") && Schema.parse(Schema.field("upload_limit_bps"),"4294967295")===4294967295)
  check("validation","port, idle and ordinary integer boundaries are enforced",null,rejected("peer_port","65536") && Schema.parse(Schema.field("peer_port"),"65535")===65535 && rejected("idle_exit_secs","29") && Schema.parse(Schema.field("idle_exit_secs"),"30")===30 && rejected("max_active_downloads","0") && rejected("timeout_secs","-1"))
  fixture.path=Quickshell.shellPath("Fixtures/settings.ndjson")
  var transcript=fixture.text().trim().split("\n").map(JSON.parse),store=Store.create(),calls=[],pending
  store.apply(transcript[1].frame,{op:"app.settings.get"})
  var s={store:store,now:100,client:{request:function(op,args,options){calls.push({op:op,args:args,options:options});return calls.length}},mutate:function(op,args,cap,callback){pending=callback;calls.push({op:op,args:args});return "setting1"}}
  var f=Flow.create(s,function(){},function(){return s.now})
  check("restart","idle settings need no clock wakeup",f.nextWakeup(),f.nextWakeup()===Infinity)
  f.set("max_active_downloads","4");pending(transcript[3].frame.result,null);store.apply(transcript[3].frame,{op:"app.settings.set"});f.frame(transcript[3].frame)
  check("confirmation","ack does not replace settings or optimistically confirm",{pending:f.pending,settings:store.settings},!!f.pending.max_active_downloads && store.settings.max_active_downloads===3 && store.settings.handler==="confirm")
  store.settings=Object.assign({},store.settings,{max_active_downloads:4});f.frame({ev:"state",topic:"settings"})
  check("confirmation","pushed settings confirm and clear pending key",f.snapshot(),!f.pending.max_active_downloads && store.settings.max_active_downloads===4)
  f.set("peer_encryption","require");pending(transcript[5].frame.result,null)
  check("restart","restart-class reply shows the exact banner",f.banner,f.banner==="applies after restart; restarting now")
  check("restart","engine restart waits for a pushed completion without polling",f.nextWakeup(),f.nextWakeup()===Infinity)
  f.frame({ev:"engine.changed",state:"running"})
  check("restart","restart completion has one minimum-display deadline",f.nextWakeup(),f.nextWakeup()===1600)
  s.now=1599;f.tick()
  check("restart","completion cannot clear the banner before the deadline",f.banner,f.banner==="applies after restart; restarting now")
  s.now=1601;f.tick()
  check("restart","finished restart needs no further wakeup",f.nextWakeup(),f.nextWakeup()===Infinity)
  check("restart","completion remains visible after the minimum display interval",f.banner,f.banner==="Settings applied after restart")
  f.set("handler","broken");check("validation","bad enum sends no write and names its error",f.errors,!!f.errors.handler)
  fixture.path=Quickshell.shellPath("Plugin/keymap.json")
  var actions=JSON.parse(fixture.text()).actions,groups=Help.groups(actions),flatten=[]
  groups.forEach(function(group){flatten=flatten.concat(group.rows)})
  check("help","all dashboard rows are grouped exactly once",flatten.length,flatten.length===actions.filter(function(a){return Array.isArray(a.keys.dashboard)}).length && new Set(flatten.map(function(a){return a.id})).size===flatten.length)
  var logs=[{seq:1,level:"info",source:"daemon"},{seq:2,level:"error",source:"engine",hash:"a".repeat(40)},{seq:3,level:"error",source:"search"}]
  check("log","level,source and hash filters preserve newest-last order",Log.select(logs,"error","engine","a".repeat(40)),JSON.stringify(Log.select(logs,"error","engine","a".repeat(40)).map(function(e){return e.seq}))==="[2]" && JSON.stringify(Log.select(logs,"error","","").map(function(e){return e.seq}))==="[2,3]")
  ;["coverage","limits","validation","confirmation","restart","help","log"].forEach(function(name){console.log("FBTQS-PASS "+name)})
  Qt.callLater(function(){Quickshell.execDetached(["qs","kill","-p",Quickshell.shellPath(".")])})
 }
}
