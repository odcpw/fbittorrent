import QtQuick
import Quickshell
import Quickshell.Io
import "Plugin/PopupState.js" as State
ShellRoot {
  function check(name,expected,actual,ok){console.log("FBTQS-STEP "+JSON.stringify({case:name,action:"evaluate real popup presentation",expect:expected,observed:JSON.stringify(actual),ok:!!ok,ms:0}))}
  Component.onCompleted:{
    var s={online:false,now:3000,client:{mode:"stopped",started:0},store:{summary:{},info:{}},torrents:[],backendState:"running"}
    var r=State.describe(s);check("states","not running, explicit Start",r,r.message.indexOf("Enter Start")>=0)
    s.client.mode="connecting";s.now=1000;r=State.describe(s);check("states","first two seconds Connecting",r,r.message==="Connecting…")
    s.startupRequested=true;r=State.describe(s);check("states","explicit Starting",r,r.message.indexOf("Starting")===0);s.startupRequested=false
    s.reconnecting=true;s.client.retryDelay=4000;r=State.describe(s);check("states","last-known rows dim while reconnecting; retry seconds",r,r.dim && r.message.indexOf("4 s")>=0);s.reconnecting=false
    s.stoppedRecently=true;r=State.describe(s);check("states","stopped recently",r,r.message.indexOf("stopped.")>=0)
    s.client.protocolMismatch=true;s.client.hello={proto:2};r=State.describe(s);check("states","protocol mismatch with restart hint",r,r.urgent && r.banner.indexOf("API 2")>=0);s.client.protocolMismatch=false
    s.online=true;s.client.hello={server:{version:"0.2.0"}};s.store.info.installed_version="0.2.1";r=State.describe(s);check("states","version skew names both versions",r,r.banner.indexOf("0.2.0 runs; 0.2.1")>=0);s.store.info={}
    s.backendState="parked";s.store.summary.network={interface:"proton0"};r=State.describe(s);check("states","VPN banner dims rows",r,r.dim && r.urgent && r.banner.indexOf("proton0")>=0)
    s.backendState="error";s.lastError="Engine restore failed";r=State.describe(s);check("states","engine failure preserves message",r,r.banner==="Engine restore failed" && r.urgent)
    s.backendState="running";s.store.summary={daemon:{upgrade_pending:true}};r=State.describe(s);check("states","upgrade pending",r,r.banner.indexOf("restarts when idle")>=0)
    s.store.summary={daemon:{idle_exit_in_s:240}};r=State.describe(s);check("states","optional idle countdown",r,r.idle==="Idle · stops in 4 min")
    s.store.summary={};r=State.describe(s);check("states","idle countdown omitted when absent; empty message stays outside list",r,r.idle==="" && r.message.indexOf("Nothing here yet")===0)
    var row={state:"downloading",progress:0.42,dl_bps:4200000,eta_s:720,peers:41,playable:{ready:true,buffered_s:240,stall_free:false,finishes_in_s:1080}}
    var meta=State.meta(row,false);check("rows","truthful ordered fields and playable warning",meta,meta.indexOf("Ready to watch · 42% · ↓ 4.2 MB/s · 12 min · 41 peers")===0 && meta.indexOf("may stall · finishes in 18 min")>=0)
    check("rows","parked omits progress",State.meta(row,true),State.meta(row,true).indexOf("42%")<0)
    check("rows","unknown peers and ETA omitted",State.meta({state:"metadata"},false),State.meta({state:"metadata"},false)==="Metadata")
    check("rows","error message and action verb",State.meta({state:"error",error:"Missing files"},false),State.meta({state:"error",error:"Missing files"},false)==="Missing files" && State.verb({op:"torrents.stop"})==="pausing…")
    console.log("FBTQS-PASS states");console.log("FBTQS-PASS rows")
    Qt.callLater(function(){Quickshell.execDetached(["qs","kill","-p",Quickshell.shellPath(".")])})
  }
}
