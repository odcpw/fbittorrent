import QtQuick
import QtTest
import Quickshell
import Quickshell.Io
import "Plugin"
import "Plugin/BarState.js" as BarState
import "Plugin/SettingsSchema.js" as Schema
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
    barConfig:Quickshell.env("FBT_TEST_BAR_CONFIG") ? JSON.parse(Quickshell.env("FBT_TEST_BAR_CONFIG")) : ({position:"top",background:"opaque",layout:{left:[],centerBegin:[],center:[],centerEnd:[],right:[{id:"oliver.torrents"}]}})
  }
  property string payload:"{}"
  TestCase {id:pointer;name:"ConvergencePointer";when:false}
  function find(item,name){if(!item)return null;if(item.objectName===name)return item;for(var i=0;i<item.children.length;i++){var match=find(item.children[i],name);if(match)return match}return null}
  function settingFields(){
    var sheet=dashboard.item ? dashboard.item.settingsSheet : null,result={}
    if(sheet)Schema.fields.forEach(function(spec){var row=sheet.row(spec.key);if(row)result[spec.key]={value:row.value,text:row.textField ? row.textField.text : null,focused:row.textField ? row.textField.activeFocus : false,pending:row.pending}})
    return result
  }
  function texts(item){if(!item)return [];var out=[];if(typeof item.text==="string" && item.visible)out.push(item.text);for(var i=0;i<item.children.length;i++)out=out.concat(texts(item.children[i]));return out}
  function torrentItem(item,hash){if(!item)return null;if(item.torrent && item.torrent.hash===hash)return item;for(var i=0;i<item.children.length;i++){var found=torrentItem(item.children[i],hash);if(found)return found}return null}
  Loader {id:dashboard;active:false;source:"Plugin/Dashboard.qml";onLoaded:{item.shell=facade;item.service=service;item.open(root.payload)}}
  IpcHandler {
    target:"shell"
    function summon(id:string,payload:string):bool{return host.summon(id,payload)}
  }
  function state(){
    var widgets=bar.moduleWidgets("oliver.torrents"),w=widgets[0]
    var shared=JSON.parse(service.stateJson()),popup=w ? w.popupItem() : null
    return {online:service.online,widgets:widgets.length,service:!!w && w.svc===service,
      width:w ? w.width : null,height:w ? w.height : null,visible:w ? w.visible : null,
      opened:w ? w.opened : null,code:dashboard.item ? dashboard.item.objectName : null,geometry:bar.debugBarGeometry(),appearance:BarState.describe(service),view:shared.view,
      settings:service.store.settings,settingFields:root.settingFields(),status:service.status,torrents:service.torrents,mutationRefs:service.client ? service.client.nextRef : 0,actions:service.client ? service.client.actions : {},
      rendered:dashboard.item ? root.texts(dashboard.item.currentView==="torrents" ? dashboard.item.torrents : dashboard.item.currentView==="rss" ? dashboard.item.rss : dashboard.item.currentView==="shows" ? dashboard.item.shows : null) : [],
      popup:popup ? {turtle:popup.turtleOn,rows:popup.rows,sections:popup.sections,cursor:popup.cursor()} : null}
  }
  IpcHandler {
    target:"fbt-dashboard"
    function ready():bool{return service.online && bar.moduleWidgets("oliver.torrents").length===1}
    function state():string{return JSON.stringify(root.state())}
    function popup():bool{var w=bar.moduleWidgets("oliver.torrents")[0];if(!w)return false;w.open();return true}
    function focusSetting(key:string):bool{return dashboard.item && dashboard.item.settingsSheet ? dashboard.item.settingsSheet.focusSetting(key) : false}
    function applySetting(key:string,text:string):bool{var sheet=dashboard.item ? dashboard.item.settingsSheet : null,row=sheet ? sheet.row(key) : null;if(!row || !row.textField || row.pending)return false;row.textField.text=text;row.textField.accepted();return true}
    function turtle():bool{var widget=bar.moduleWidgets("oliver.torrents")[0],button=widget ? root.find(widget.popupItem(),"popup-turtle") : null;if(!button || !button.enabled)return false;pointer.mouseClick(button,button.width/2,button.height/2,Qt.LeftButton);return true}
    function popupPause():bool{var widget=bar.moduleWidgets("oliver.torrents")[0],popup=widget ? widget.popupItem() : null;if(!popup || !popup.rows.length)return false;var row=root.torrentItem(popup,popup.rows[0].hash);if(!row)return false;pointer.mouseClick(row,row.width/2,row.height/2,Qt.MiddleButton);return true}
    function grab(path:string):bool{var w=bar.moduleWidgets("oliver.torrents")[0];return w ? w.grabToImage(function(result){result.saveToFile(path)}) : false}
  }
  Component.onCompleted:registry.register("oliver.torrents",widgetComponent,{firstParty:false})
}
