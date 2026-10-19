import QtQuick
import Quickshell
import Quickshell.Io
import "Plugin"
import "Plugin/Focus.js" as Focus
import "Host"

ShellRoot {
  id:root
  property var config:({bar:{layout:{right:["oliver.torrents"]}},plugins:[]})
  property var manifest:({})
  property string payload:"{}"
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
    input.path=Quickshell.shellPath("Plugin/manifest.json")
    manifest=registry.validateManifest(JSON.parse(input.text()),Quickshell.shellPath("Plugin"))
    manifest.__sourceDir=Quickshell.shellPath("Plugin")
    registry.installedPlugins={"oliver.torrents":manifest}
  }
}
