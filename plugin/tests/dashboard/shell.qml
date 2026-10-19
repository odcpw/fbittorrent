import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons as Commons
import "Plugin"
import "Plugin/Focus.js" as Focus
import "Host"

ShellRoot {
  id:root
  property var config:({bar:{layout:{right:["oliver.torrents"]}},plugins:[]})
  property var manifest:({})
  property string payload:"{}"
  property bool scrolling:false
  property int scrollCount:0
  property real scrollStarted:0
  property real scrollElapsed:0
  Timer {
    interval:16;repeat:true;running:root.scrolling
    onTriggered:{
      if(!service.dashboardOwner){root.scrolling=false;return}
      // QML Timer follows the animation clock. Schedule 50 ms input phases
      // from elapsed time rather than accumulating its frame quantization.
      var due=Math.min(200,Math.floor((Date.now()-root.scrollStarted)/50))
      while(root.scrollCount<due){
        service.dashboardOwner.scroll((Math.floor(root.scrollCount/50)%2===0) ? 112 : -112)
        root.scrollCount++
      }
      if(root.scrollCount===200){root.scrollElapsed=Date.now()-root.scrollStarted;root.scrolling=false}
    }
  }
  FileView {id:input;blockLoading:true;blockAllReads:true;preload:false}
  PluginRegistry {
    id:registry
    // Use the installed validation/routing implementation without starting a watcher.
    function ensureUserDir() {}
    shellConfigProvider:function(){return root.config}
  }
  Service {id:service;settings:({binary:Quickshell.env("FBT_TEST_BINARY")})}
  PanelWindow {visible:false;screen:Quickshell.screens[0];implicitWidth:32;implicitHeight:32;BarWidget {shell:facade}}
  QtObject {
    id:facade
    function serviceFor(id){return id==="oliver.torrents" ? service : null}
    function summon(id,args) {
      if(!registry.isEnabled(id) || root.manifest.kinds.indexOf("panel")<0)return false
      root.payload=args || "{}"
      if(dashboard.item)return dashboard.item.open(root.payload)
      dashboard.active=true;return true
    }
    function hide(id) {if(id!=="oliver.torrents")return false;dashboard.active=false;return true}
  }
  Loader {
    id:dashboard
    active:false
    source:root.manifest.entryPoints ? "Plugin/"+root.manifest.entryPoints.panel : ""
    onLoaded:{item.shell=facade;item.service=service;item.manifest=root.manifest;item.open(root.payload)}
  }
  IpcHandler {
    target:"shell"
    function summon(id:string,payload:string):bool{return facade.summon(id,payload)}
    function hide(id:string):bool{return facade.hide(id)}
  }
  IpcHandler {
    target:"fbt-dashboard"
    function fileEditing():bool{return dashboard.item && dashboard.item.torrents && dashboard.item.torrents.pane ? dashboard.item.torrents.pane.editing : false}
    function logEntries():string{return JSON.stringify(service.store.logEntries(service.detailFlow ? service.detailFlow.hash : ""))}
    function confirmed(hash:string):string{return JSON.stringify(service.store.torrents[hash] || null)}
    function activeWindow():bool{return dashboard.item ? dashboard.item.isWindowActive() : false}
    function startScrolling():bool {
      if(Quickshell.env("FBT_TEST_HOOKS")!=="1" || !service.dashboardOwner)return false
      root.scrollCount=0;root.scrollStarted=Date.now();root.scrollElapsed=0;root.scrolling=true;return true
    }
    function scrollStats():string{return JSON.stringify({count:root.scrollCount,elapsed_ms:root.scrollElapsed,active:root.scrolling})}
    function routeDrop(url:string):bool {if(Quickshell.env("FBT_TEST_HOOKS")!=="1" || !dashboard.item)return false;return dashboard.item.acceptDrop([url],"")}
    function rowPoint(index:int):string{return JSON.stringify(dashboard.item ? dashboard.item.rowPoint(index) : null)}
    function resize(width:int,height:int):bool{if(!dashboard.item)return false;dashboard.item.resize(width,height);return true}
    function scroll(dy:real):bool{if(!dashboard.item)return false;dashboard.item.scroll(dy);return true}
    function grab(path:string):bool{if(!dashboard.item)return false;return dashboard.item.grab(path)}
    function state():string{return service.stateJson()}
    function focusCommand(address:string,lua:bool):string{return Focus.command(address,lua) || "refused"}
    function ready():bool{return service.online && registry.isEnabled("oliver.torrents")}
    function visibility():string{return JSON.stringify({service:service.dashboardVisible,client:service.client ? service.client.dashboardVisible : false})}
    function pendingRequests():int{return service.client ? Object.keys(service.client.pending).length+service.client.queue.length : 0}
    function focusSteps():int{return service.windowFocusSteps}
    function registryState():string{return JSON.stringify({enabled:registry.isEnabled("oliver.torrents"),location:registry.findEntryLocation(root.config,"oliver.torrents"),kinds:root.manifest.kinds})}
    function summonView(payload:string):string {
      if(Quickshell.env("FBT_TEST_HOOKS")!=="1")return "refused: FBT_TEST_HOOKS=1 required"
      return facade.summon("oliver.torrents",payload) ? "ok" : "failed"
    }
  }
  Component.onCompleted:{
    if(Quickshell.env("FBT_FONT_FAMILY"))Commons.Style.fontFamily=Quickshell.env("FBT_FONT_FAMILY")
    input.path=Quickshell.shellPath("Plugin/manifest.json")
    manifest=registry.validateManifest(JSON.parse(input.text()),Quickshell.shellPath("Plugin"))
    manifest.__sourceDir=Quickshell.shellPath("Plugin")
    registry.installedPlugins={"oliver.torrents":manifest}
  }
}
