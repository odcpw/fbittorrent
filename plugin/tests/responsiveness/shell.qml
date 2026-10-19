import QtQuick
import QtTest
import Quickshell
import Quickshell.Io
import qs.Commons as Commons
import "Plugin"
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
    _hide:function(id){dashboard.active=false;return true}
  }
  QtObject {
    id:host
    function pluginShellForId(id){return id==="oliver.torrents" ? facade : null}
    function summon(id,payload){if(id!=="oliver.torrents")return false;root.start();root.payload=payload || "{}";if(dashboard.item)return dashboard.item.open(root.payload);dashboard.active=true;return true}
  }
  HostBar.Bar {
    id:bar;shell:host;barWidgetRegistry:registry
    barConfig:({position:"top",background:"opaque",layout:{left:[],centerBegin:[],center:[],centerEnd:[],right:[{id:"oliver.torrents"}]}})
  }
  property string payload:"{}"
  readonly property var widget:bar.moduleWidgets("oliver.torrents")[0] || null
  readonly property var content:widget ? widget.popupItem() : null
  Loader {id:dashboard;active:false;source:"Plugin/Dashboard.qml";onLoaded:{item.shell=facade;item.service=service;item.open(root.payload)}}
  property var pending:null
  property var samples:[]
  property var rememberedTable:null
  function start(){if(pending && !pending.at)pending.at=Date.now()}
  function find(item,name){if(!item)return null;if(item.objectName===name)return item;for(var i=0;i<item.children.length;i++){var child=find(item.children[i],name);if(child)return child}return null}
  function readySample(withData){
    if(!pending)return false
    var d=dashboard.item,v=service.dashboardState || {},spec=pending.spec
    if(spec.kind==="popup")return !!content && widget.opened && content.delegateCount>0
    if(!d || !d.perfWindow.visible)return false
    if(spec.kind==="open")return true
    if(spec.kind==="rows")return v.rows===spec.count
    if(spec.kind==="view")return d.currentView===spec.view && (spec.view==="torrents" ? !!d.torrents : spec.view==="search" ? !!d.search : spec.view==="shows" ? !!d.shows : spec.view==="rss" ? !!d.rss : spec.view==="settings" ? !!d.settingsSheet : !!d.logView)
    if(spec.kind==="detail"){
      if(!d.torrents || !d.torrents.pane || service.detailFlow.hash!==spec.hash || service.detailFlow.tab!==spec.tab)return false
      if(!spec.data || !withData)return true
      if(spec.tab==="files")return !service.detailFlow.loading && service.detailFlow.files.length>0
      if(spec.tab==="peers" || spec.tab==="trackers")return Object.keys(service.store.topics["torrent:"+spec.hash+"."+spec.tab] || {}).length>0
      if(spec.tab==="pieces")return !!d.torrents.pane.piecesState && d.torrents.pane.piecesState.cells>0
      if(spec.tab==="log")return service.store.topics.log!==undefined
      return !!service.detailFlow.row() && service.detailFlow.row().file_count!==undefined
    }
    if(spec.kind==="filter")return v.query===spec.text && v.rows===spec.count
    if(spec.kind==="search")return !!d.search && d.search.queryTarget.text===spec.text
    if(spec.kind==="review")return !!v.add && v.add.visible && v.add.field==="sheet" && v.add.name===spec.name
    return false
  }
  function presented(popup){
    if(!pending || !pending.at || (pending.spec.kind==="popup")!==popup)return
    var complete=readySample(true),feedback=pending.spec.kind==="detail" && pending.spec.data && !pending.feedback && readySample(false)
    if(!complete && !feedback)return
    var d=dashboard.item,stats=d ? d.perfPublish : {calls:0,ms:0}
    var sample={name:pending.name,ms:Date.now()-pending.at,frame_at:Date.now(),spec:pending.spec,
      publications:stats.calls-pending.publications,publication_ms:stats.ms-pending.publicationMs,
      api_frames:service.perfFrames-pending.frames,row_syncs:service.perfSyncRows-pending.rowSyncs,
      rows:d && d.torrents ? d.torrents.snapshot().rows : 0}
    if(feedback){samples.push(Object.assign({},sample,{name:pending.name+"_frame"}));pending.feedback=true}
    if(!complete)return
    if(pending.spec.kind==="open"){
      if(!pending.first){samples.push(Object.assign({},sample));pending.first=true}
      if(sample.rows!==pending.spec.count)return
      sample.name="dashboard_usable"
    }
    samples.push(sample)
    pending=null
  }
  Connections {target:dashboard.item ? dashboard.item.perfWindow.contentItem.Window.window : null;function onFrameSwapped(){root.presented(false)}}
  Connections {target:root.content ? root.content.Window.window : null;function onFrameSwapped(){root.presented(true)}}
  TestCase {id:input;name:"ResponsivenessInput";when:false}
  // Same socket and dispatcher as the installed shell; omarchy-shell uses it
  // before starting a separate qs IPC process.
  SocketServer {
    active:true
    path:Quickshell.env("XDG_RUNTIME_DIR")+"/omarchy-shell-"+Qt.md5(Quickshell.env("OMARCHY_PATH")+"/shell\n"+Quickshell.env("WAYLAND_DISPLAY")).slice(0,16)+".sock"
    handler:Socket {
      id:connection
      parser:SplitParser {splitMarker:"\u001e";onRead:function(data){var fields=String(data).split("\u001f"),result=fields.length>=2 ? Commons.IpcRegistry.call(fields[0],fields[1],fields.slice(2)) : {ran:false};connection.write(result.ran ? "OK\u001f"+result.output+"\u001e" : "SKIP\u001e");connection.flush();connection.connected=false}}
    }
  }
  Commons.ShellIpc {target:"shell";function summon(id:string,payload:string):bool{return host.summon(id,payload)}}
  Commons.ShellIpc {
    target:"fbt-dashboard"
    function ready():bool{return service.online && !!root.widget}
    function arm(name:string,json:string):bool{if(root.pending)return false;var d=dashboard.item,stats=d ? d.perfPublish : {calls:0,ms:0};root.pending={name:name,at:0,spec:JSON.parse(json),publications:stats.calls,publicationMs:stats.ms,frames:service.perfFrames,rowSyncs:service.perfSyncRows};return true}
    function samples():string{return JSON.stringify(root.samples)}
    function done():bool{return root.pending===null}
    function state():string{return service.stateJson()}
    function hide():bool{if(root.widget)root.widget.close();dashboard.active=false;return true}
    function popup():bool{root.start();root.widget.open();return true}
    function closePopup():bool{root.widget.close();return true}
    function key(key:int,modifiers:int):bool{if(!dashboard.item)return false;root.start();var value=key<256 ? String.fromCharCode(key) : key;if(typeof value==="string" && !(modifiers & Qt.ShiftModifier))value=value.toLowerCase();input.keyClick(value,modifiers,0);return true}
    function tab(name:string):bool{var button=root.find(dashboard.item ? dashboard.item.torrents.pane : null,"details-tab-"+name);if(!button)return false;root.start();input.mouseClick(button,button.width/2,button.height/2,Qt.LeftButton,Qt.NoModifier,0);return true}
    function focus():bool{if(!dashboard.item)return false;dashboard.item.focusView();return true}
    function focusState():string{return JSON.stringify({pending:!!service.windowFocus,active:dashboard.item ? dashboard.item.perfWindow.contentItem.Window.window.active : false})}
    function rememberTable():bool{root.rememberedTable=dashboard.item ? dashboard.item.torrents : null;return !!root.rememberedTable}
    function tableState():string{return JSON.stringify({same:!!dashboard.item && root.rememberedTable===dashboard.item.torrents,visible:dashboard.item && dashboard.item.torrents ? dashboard.item.torrents.visible : false,topics:service.detailTopics})}
    function counts():string{return JSON.stringify({syncRows:service.perfSyncRows,frames:service.perfFrames,popupDelegates:root.content ? root.content.delegateCount : 0,pending:service.client.state().pending})}
  }
  Component.onCompleted:registry.register("oliver.torrents",widgetComponent,{firstParty:false})
}
