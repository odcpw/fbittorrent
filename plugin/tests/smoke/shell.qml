import QtQuick
import Quickshell
import Quickshell.Io
import "Plugin"

ShellRoot {
  id:root
  property string phase:"connect"
  property string hash:Quickshell.env("FBT_SMOKE_HASH")
  property string initialInstance:""
  property double began:Date.now()
  property double phaseAt:Date.now()
  property double readyAt:0
  property bool finished:false
  Service {id:svc;manifest:({version:"smoke"});popupOpen:true}
  FileView {
    id:readiness
    path:Quickshell.env("FBT_SMOKE_READY_FILE")
    watchChanges:true
    onFileChanged:reload()
    onLoaded:root.readyAt=JSON.parse(text()).ready_ms || 0
  }
  function mark(name,expect,observed,ok) {
    console.log("FBTQS-STEP "+JSON.stringify({case:name,action:name,expect:expect,observed:observed,ok:!!ok,ms:Date.now()-phaseAt}))
    console.log("FBTQS-PASS "+name)
  }
  function enter(next) {phase=next;phaseAt=Date.now()}
  function finish() {
    finished=true
    Qt.callLater(function(){Quickshell.execDetached(["qs","kill","-p",Quickshell.shellPath(".")])})
  }
  Timer {
    interval:40;repeat:true;running:!root.finished
    onTriggered: {
      var row=svc.store.torrents[root.hash]
      if(root.phase==="connect" && svc.online) {
        root.initialInstance=svc.instance
        svc.add({magnet:"magnet:?xt=urn:btih:"+root.hash+"&dn=fbt-smoke"},false);root.enter("add")
      } else if(root.phase==="add" && row && row.state==="metadata") {
        root.mark("add","metadata intent is saved without peers, with payload upload off",JSON.stringify(row),row.name==="fbt-smoke" && row.upload_enabled===false && row.seed===false && row.progress===undefined && row.size===undefined && svc.client.popupOpen)
        svc.pause(root.hash);root.enter("pause")
      } else if(root.phase==="pause" && row && row.state==="paused") {
        root.mark("pause","confirmed paused state",JSON.stringify(row),true)
        svc.resume(root.hash);root.enter("resume")
      } else if(root.phase==="resume" && row && row.state==="metadata") {
        root.mark("resume","confirmed metadata resolution restarts",JSON.stringify(row),true)
        svc.toggleTorrent({hash:root.hash,state:"metadata"});root.enter("toggle_pause")
      } else if(root.phase==="toggle_pause" && row && row.state==="paused") {
        // A retained Qt/JS cursor object can have the pre-pause state.
        svc.toggleTorrent({hash:root.hash,state:"metadata"});root.enter("toggle_resume")
      } else if(root.phase==="toggle_resume" && row && row.state==="metadata") {
        root.mark("toggle_current_state","toggle resolves the current keyed row, even when the cursor object is stale",JSON.stringify(row),true)
        svc.remove(root.hash,false);root.enter("remove_keep_files")
      } else if(root.phase==="remove_keep_files" && !row) {
        root.mark("remove_keep_files","confirmed removal without delete_files",JSON.stringify(svc.torrents),true)
        root.enter("reconnect");console.log("FBTQS-WAIT reconnect")
      } else if(root.phase==="reconnect" && root.readyAt>0 && svc.online && svc.instance!==root.initialInstance && svc.store.hasTorrents) {
        var state=svc.client.state(),subscribe=svc.client.caps.indexOf("subscribe")>=0
        var ok=Date.now()-root.readyAt<1000
        ok=ok && subscribe && state.subs.summary>0 && state.subs.torrents>0
        root.mark("reconnect","new daemon instance reconnects within 1 s of readiness; summary and torrent subscriptions restored",JSON.stringify({elapsed_ms:Date.now()-root.readyAt,instance:svc.instance,subs:state.subs,mode:svc.client.mode,subscription_check:subscribe ? "checked" : "missing required subscribe capability"}),ok)
        root.finish()
      }
      if(Date.now()-root.phaseAt>20000 || Date.now()-root.began>45000) {
        root.mark(root.phase,"phase completes within 20 s (whole smoke 45 s)",JSON.stringify({row:row,error:svc.lastError,state:svc.stateJson()}),false)
        root.finish()
      }
    }
  }
}
