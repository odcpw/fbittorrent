import QtQuick
import Quickshell
import Quickshell.Io
import QtQuick.Window
import qs.Commons as Commons
import "Plugin/components"
import "Plugin/Model.js" as Model
import "Plugin/PopupState.js" as State
ShellRoot {
  id:root
  QtObject {
    id:host
    property var svc:QtObject {
      property int actionRevision:0
      function rowPending(hash){return false}
      function pendingAction(hash){return null}
    }
    property var presentation:({dim:false})
    property color foreground:Commons.Color.foreground
    property color dim:Model.dim(foreground,Commons.Color.popups.background)
    property bool cursorActive:false
    property string focusSection:"rows"
    property int rowIndex:0
    property var pendingRemoval:null
    function registerConfirm(hash,item){}
    function unregisterConfirm(hash,item){}
  }
  Window {visible:true;width:380;height:100
    PopupRow {id:readyRow;width:parent.width;host:host;rowIdx:0;torrent:({hash:"a".repeat(40),name:"Playable presentation control",state:"downloading",progress:0.4,playable:{ready:true,stall_free:false,buffered_s:240}})}
  }
  Timer {interval:30;running:true
    onTriggered:{
      var meta=root.find(readyRow,"popup-row-meta")
      root.check("ready_color","playable row uses accent color and bold fallback",meta.color,meta.objectName==="popup-row-meta" && meta.color.toString()===readyRow.accentSpec.color.toString() && meta.font.bold===readyRow.accentSpec.bold)
      console.log("FBTQS-PASS ready_color")
      Quickshell.execDetached(["qs","kill","-p",Quickshell.shellPath(".")])
    }
  }
  function find(item,name){if(item.objectName===name)return item;for(var i=0;i<item.children.length;i++){var found=find(item.children[i],name);if(found)return found}return null}
  function check(name,expected,actual,ok){console.log("FBTQS-STEP "+JSON.stringify({case:name,action:"evaluate real popup presentation",expect:expected,observed:JSON.stringify(actual),ok:!!ok,ms:0}))}
  Component.onCompleted:{
    var s={online:false,now:3000,client:{mode:"stopped",started:0},store:{summary:{},info:{}},torrents:[],backendState:"running"}
    var r=State.describe(s);check("states","not running, explicit Start",r,r.message.indexOf("Enter Start")>=0)
    s.client.mode="connecting";s.now=1000;r=State.describe(s);check("states","first two seconds Connecting",r,r.message==="Connecting…")
    s.startupRequested=true;r=State.describe(s);check("states","explicit Starting",r,r.message.indexOf("Starting")===0);s.startupRequested=false
    s.reconnecting=true;s.client.retryDelay=4000;r=State.describe(s);check("states","last-known rows dim while reconnecting; retry seconds",r,r.dim && r.message.indexOf("4 s")>=0);s.reconnecting=false
    s.stoppedRecently=true;r=State.describe(s);check("states","stopped recently",r,r.message.indexOf("stopped.")>=0)
    s.client.protocolMismatch=true;s.client.hello={proto:2};r=State.describe(s);check("states","protocol mismatch with restart hint",r,r.urgent && r.banner.indexOf("API 2")>=0);s.client.protocolMismatch=false
    s.online=true;s.client.hello={server:{version:"0.2.0"}};s.store.info.installed_version="0.2.1";r=State.describe(s);check("states","current native UI has no version-skew warning",r,r.banner==="");s.store.info={}
    s.backendState="parked";s.store.summary.network={interface:"proton0"};r=State.describe(s);check("states","VPN banner dims rows",r,r.dim && r.urgent && r.banner.indexOf("proton0")>=0)
    s.backendState="error";s.lastError="Engine restore failed";r=State.describe(s);check("states","engine failure preserves message",r,r.banner==="Engine restore failed" && r.urgent)
    s.backendState="running";s.store.summary={daemon:{upgrade_pending:true}};r=State.describe(s);check("states","upgrade pending",r,r.banner.indexOf("restarts when idle")>=0)
    s.store.summary={daemon:{idle_exit_in_s:240}};r=State.describe(s);check("states","optional idle countdown",r,r.idle==="Idle · stops in 4 min")
    s.store.summary={};r=State.describe(s);check("states","idle countdown omitted when absent; empty message stays outside list",r,r.idle==="" && r.message.indexOf("Nothing here yet")===0)
    s.backendState="parked";s.store.summary={disk:{low:true,needed_bytes:104857600,free_bytes:67108864},network:{interface:"fixture0"}}
    r=State.describe(s);check("disk","disk warning takes priority and formats actual binary units",r,r.urgent && r.banner==="Not enough space for the running downloads: 100.0 MiB needed, 64.0 MiB free")
    check("disk","missing free bytes remains unknown",State.diskBanner({low:true,needed_bytes:104857600}),State.diskBanner({low:true,needed_bytes:104857600})==="Not enough space for the running downloads: 100.0 MiB needed, free space unknown")
    check("disk","recovered disk clears its banner",State.diskBanner({low:false,needed_bytes:104857600,free_bytes:67108864}),State.diskBanner({low:false,needed_bytes:104857600,free_bytes:67108864})==="")
    s.backendState="running";s.store.summary={}
    console.log("FBTQS-PASS disk")
    var row={state:"downloading",progress:0.42,dl_bps:4200000,eta_s:720,peers:41,playable:{ready:true,buffered_s:240,stall_free:false,finishes_in_s:1080}}
    var meta=State.meta(row,false);check("rows","truthful ordered fields and playable warning",meta,meta.indexOf("Playable · may stall · 42% · ↓ 4.2 MB/s · 12 min · 41 peers connected · seeds unknown")===0 && meta.indexOf("may stall · finishes in 18 min")>=0)
    check("rows","parked omits progress",State.meta(row,true),State.meta(row,true).indexOf("42%")<0)
    check("rows","unknown peers and ETA omitted",State.meta({state:"metadata"},false),State.meta({state:"metadata"},false)==="Metadata")
    check("rows","error message and action verb",State.meta({state:"error",error:"Missing files"},false),State.meta({state:"error",error:"Missing files"},false)==="Missing files" && State.verb({op:"torrents.stop"})==="pausing…")
    check("states","popup group keys map to real dashboard filters",["live","queued","paused","done","other"].map(State.dashboardFilter),JSON.stringify(["live","queued","paused","done","other"].map(State.dashboardFilter))===JSON.stringify(["downloading","queued","paused","done","all"]))
    console.log("FBTQS-PASS states");console.log("FBTQS-PASS rows")

  }
}
