import QtQuick
import Quickshell
import Quickshell.Io
import "Plugin"
import "Plugin/BarState.js" as BarState
import "Host"
import "HostBar" as HostBar

ShellRoot {
  id:root
  Service {id:service;shell:facade;settings:({binary:Quickshell.env("FBT_TEST_BINARY")})}
  BarWidgetRegistry {id:registry}
  Component {id:widgetComponent;BarWidget {}}
  PluginShellApi {
    id:facade;pluginId:"oliver.torrents"
    _serviceLookup:function(id){return id==="oliver.torrents" ? service : null}
    _summon:function(id,payload){return host.summon(id,payload)}
    _hide:function(id){if(dashboard.item)dashboard.item.close();dashboard.active=false;return true}
  }
  QtObject {
    id:host
    function pluginShellForId(id){return id==="oliver.torrents" ? facade : null}
    function summon(id,payload){if(id!=="oliver.torrents")return false;root.payload=payload || "{}";if(dashboard.item)return dashboard.item.open(root.payload);dashboard.active=true;return true}
  }
  HostBar.Bar {
    id:bar;shell:host;barWidgetRegistry:registry
    barConfig:({position:"top",background:"opaque",layout:{left:[],centerBegin:[],center:[],centerEnd:[],right:[{id:"oliver.torrents"}]}})
  }
  property string payload:"{}"
  Loader {id:dashboard;active:false;source:"Plugin/Dashboard.qml";onLoaded:{item.shell=facade;item.service=service;item.open(root.payload)}}
  IpcHandler {
    target:"shell"
    function summon(id:string,payload:string):bool{return host.summon(id,payload)}
  }
  function state(){
    var widgets=bar.moduleWidgets("oliver.torrents"),w=widgets[0]
    return {online:service.online,widgets:widgets.length,service:!!w && w.svc===service,
      width:w ? w.width : null,height:w ? w.height : null,visible:w ? w.visible : null,
      opened:w ? w.opened : null,code:dashboard.item ? dashboard.item.objectName : null,geometry:bar.debugBarGeometry(),appearance:BarState.describe(service),view:service.dashboardState}
  }
  IpcHandler {
    target:"fbt-dashboard"
    function ready():bool{return service.online && bar.moduleWidgets("oliver.torrents").length===1}
    function state():string{return JSON.stringify(root.state())}
    function popup():bool{var w=bar.moduleWidgets("oliver.torrents")[0];if(!w)return false;w.open();return true}
    function grab(path:string):bool{var w=bar.moduleWidgets("oliver.torrents")[0];return w ? w.grabToImage(function(result){result.saveToFile(path)}) : false}
  }
  Component.onCompleted:registry.register("oliver.torrents",widgetComponent,{firstParty:false})
}
