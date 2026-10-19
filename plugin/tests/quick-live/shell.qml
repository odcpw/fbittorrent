import QtQuick
import Quickshell
import Quickshell.Io
import "Plugin"
import "QuickPlugin"
import "Host"
ShellRoot {
 id:root
 property var mainManifest:({})
 property var quickManifest:({})
 property var config:({bar:{layout:{right:["oliver.torrents"]}},plugins:[{id:"oliver.torrents-quick"}]})
 FileView {id:document;blockLoading:true;blockAllReads:true}
 FileView {id:quickDocument;blockLoading:true;blockAllReads:true}
 PluginRegistry {id:registry;function ensureUserDir(){}
 shellConfigProvider:function(){return root.config}}
 Service {id:svc;settings:({binary:Quickshell.env("FBT_TEST_BINARY")})}
 Dashboard {id:dashboard;service:svc;shell:facade}
 Quick {id:quick;shell:facade}
 PanelWindow {visible:true;implicitWidth:32;implicitHeight:32;BarWidget {id:widget;shell:facade}}
 QtObject {
  id:facade
  function serviceFor(id){return id==="oliver.torrents" ? svc : null}
  function summon(id,payload){
   if(!registry.isEnabled(id))return false
   if(id==="oliver.torrents-quick" && root.quickManifest.kinds.indexOf("overlay")>=0)return quick.open(payload)
   if(id==="oliver.torrents" && root.mainManifest.kinds.indexOf("panel")>=0)return dashboard.open(payload)
   return false
  }
  function hide(id){if(id==="oliver.torrents-quick"){quick.close();return true}if(id==="oliver.torrents"){widget.close();dashboard.close();return true}return false}
  function toggle(id){if(id==="oliver.torrents-quick"){quick.toggle();return true}if(id==="oliver.torrents"){widget.toggle();return true}return false}
 }
 IpcHandler {target:"shell"
  function summon(id:string,payload:string):bool{return facade.summon(id,payload)}
  function hide(id:string):bool{return facade.hide(id)}
  function toggle(id:string,payload:string):bool{return facade.toggle(id)}
 }
 IpcHandler {target:"quick-test"
  function state():string{return JSON.stringify({online:svc.online,manifests:{main:root.mainManifest,quick:root.quickManifest,config:root.config},palette:quick.viewModel,visible:quick.opened,query:quick.queryText,focused:quick.inputFocused,bar:widget.opened,dashboard:svc.dashboardState,add:svc.addFlow ? svc.addFlow.snapshot() : null,client:svc.client ? svc.client.state() : null})}
  function ready():bool{return svc.online && registry.isEnabled("oliver.torrents-quick")}
  function closeDashboard():bool{dashboard.close();return true}
 }
 Component.onCompleted:{
  document.path=Qt.resolvedUrl("Plugin/manifest.json");mainManifest=registry.validateManifest(JSON.parse(document.text()),Quickshell.shellPath("Plugin"))
  quickDocument.path=Qt.resolvedUrl("QuickPlugin/manifest.json");quickManifest=registry.validateManifest(JSON.parse(quickDocument.text()),Quickshell.shellPath("QuickPlugin"))
  var installed={};installed[mainManifest.id]=mainManifest;installed[quickManifest.id]=quickManifest;registry.installedPlugins=installed
 }
}
