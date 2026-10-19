import QtQuick
import QtTest
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
  TestCase {id:preferencesInput;name:"PreferencesPointer";when:false}
  function find(item,name){if(!item)return null;if(item.objectName===name)return item;for(var i=0;i<item.children.length;i++){var child=find(item.children[i],name);if(child)return child}return null}
  function texts(item){var out=[];if(typeof item.text==="string" && item.visible)out.push(item.text);for(var i=0;i<item.children.length;i++)out=out.concat(texts(item.children[i]));return out}
  function tableRows(item){
    if(item.hasCursor!==undefined && item.torrent!==undefined)return [{hash:item.torrent.hash,texts:texts(item)}]
    var out=[];for(var i=0;i<item.children.length;i++)out=out.concat(tableRows(item.children[i]));return out
  }
  IpcHandler {
    target:"fbt-dashboard"
    function renderedRows():string{return JSON.stringify(dashboard.item && dashboard.item.torrents ? root.tableRows(dashboard.item.torrents) : [])}
    function detailTexts():string{return JSON.stringify(dashboard.item && dashboard.item.torrents && dashboard.item.torrents.pane ? root.texts(dashboard.item.torrents.pane) : [])}
    function setting(key:string,valueJson:string):string {
      if(Quickshell.env("FBT_TEST_HOOKS")!=="1" || !dashboard.item || !dashboard.item.settingsSheet)return "refused"
      var sheet=dashboard.item.settingsSheet,item=sheet.row(key),value=JSON.parse(valueJson)
      if(!item)return "unknown key"
      sheet.focusSetting(key)
      if(item.spec.kind==="bool")preferencesInput.mouseClick(item.toggle,item.toggle.width/2,item.toggle.height/2,Qt.LeftButton)
      else if(item.spec.kind==="choice" || item.spec.kind==="interface")item.dropdown.changed(value===null ? "" : String(value))
      else{item.textField.text=value===null ? "" : String(value);item.textField.accepted()}
      return "ok"
    }
    function focusSetting(key:string):bool {return Quickshell.env("FBT_TEST_HOOKS")==="1" && dashboard.item && dashboard.item.settingsSheet ? dashboard.item.settingsSheet.focusSetting(key) : false}
    function settingsTrace():string {return JSON.stringify({actions:service.client ? service.client.actions : {},results:service.store.mutations})}
    function recheckState():string {
      var button=root.find(dashboard.item ? dashboard.item.torrents : null,"torrent-recheck"),overview=root.find(dashboard.item ? dashboard.item.torrents : null,"torrent-overview")
      return JSON.stringify({button:button ? {text:button.text,enabled:button.enabled,y:button.mapToItem(overview,0,0).y,height:button.height} : null,viewport:overview ? {height:overview.height,moving:overview.contentItem.moving,contentY:overview.contentItem.contentY,contentHeight:overview.contentItem.contentHeight} : null,error:service.lastError,message:service.lastMessageText,actions:service.client ? service.client.actions : {}})
    }
    function scrollRecheck():bool {
      if(Quickshell.env("FBT_TEST_HOOKS")!=="1" || !dashboard.item)return false
      var overview=root.find(dashboard.item.torrents,"torrent-overview")
      if(!overview)return false
      preferencesInput.mouseWheel(overview,overview.width/2,overview.height/2,0,-4800);return true
    }
    function recheckButton():bool {
      if(Quickshell.env("FBT_TEST_HOOKS")!=="1" || !dashboard.item)return false
      var button=root.find(dashboard.item.torrents,"torrent-recheck");if(!button || !button.enabled)return false
      preferencesInput.mouseClick(button,button.width/2,button.height/2,Qt.LeftButton);return true
    }
    function helpKeys():string{return JSON.stringify(dashboard.item && dashboard.item.helpOverlay ? dashboard.item.helpOverlay.listedKeys() : [])}
    function logRows():string{return JSON.stringify(dashboard.item && dashboard.item.logView ? dashboard.item.logView.rendered() : [])}
    function logFilter(kind:string,value:string):bool {
      if(Quickshell.env("FBT_TEST_HOOKS")!=="1" || !dashboard.item || !dashboard.item.logView || ["level","source"].indexOf(kind)<0)return false
      var button=root.find(dashboard.item.logView,"log-"+kind+"-"+(value || "all"));if(!button)return false
      preferencesInput.mouseClick(button,button.width/2,button.height/2,Qt.LeftButton);return true
    }
    function showsDropdown(name:string):bool {
      if(Quickshell.env("FBT_TEST_HOOKS")!=="1" || !dashboard.item || !dashboard.item.shows || ["read","quality","preference"].indexOf(name)<0)return false
      var dropdown=root.find(dashboard.item.shows,"shows-"+name);if(!dropdown)return false
      preferencesInput.mouseClick(dropdown,dropdown.width/2,dropdown.height-Commons.Style.spacing.controlHeight/2,Qt.LeftButton);return true
    }
    function detailTab(name:string):bool {
      if(Quickshell.env("FBT_TEST_HOOKS")!=="1" || !dashboard.item || !dashboard.item.torrents)return false
      var button=root.find(dashboard.item.torrents.pane,"details-tab-"+name);if(!button)return false
      preferencesInput.mouseClick(button,button.width/2,button.height/2,Qt.LeftButton);return true
    }
    function gridState():string {
      var view=dashboard.item ? dashboard.item.search : null
      return JSON.stringify(view ? {grid:view.gridShowing ? view.viewState.grid : null,query:view.queryTarget.text,mode:view.viewModel.mode,label:view.viewModel.label,phase:view.viewModel.phase,total:view.viewModel.total,cursor:view.viewModel.cursor,pool:service.browsePosters ? service.browsePosters.state() : null} : {})
    }
    function eventTrace():string{return JSON.stringify(service.store.events)}
    function searchFocus():string {
      var view=dashboard.item ? dashboard.item.search : null,query=view ? view.queryTarget : null
      return JSON.stringify({search:!!view,active:view ? view.activeFocus : null,visible:view ? view.visible : null,query:query ? {active:query.activeFocus,focus:query.focus,text:query.text,visible:query.visible} : null,state:service.dashboardState})
    }
    function trackerRows():string {
      if(Quickshell.env("FBT_TEST_HOOKS")!=="1" || !dashboard.item)return "[]"
      var view=root.find(dashboard.item.torrents,"tracker-view");return JSON.stringify(view ? view.renderedRows() : [])
    }
    function trackerReannounce():bool {
      if(Quickshell.env("FBT_TEST_HOOKS")!=="1" || !dashboard.item)return false
      var button=root.find(dashboard.item.torrents,"trackers-reannounce");if(!button || !button.enabled)return false
      preferencesInput.mouseClick(button,button.width/2,button.height/2,Qt.LeftButton);return true
    }
    function trackerButton(kind:string):bool {
      if(Quickshell.env("FBT_TEST_HOOKS")!=="1" || !dashboard.item || ["add","remove","add-cancel","add-submit"].indexOf(kind)<0)return false
      var button=root.find(dashboard.item.torrents,kind.indexOf("add-")===0 ? "tracker-"+kind : "trackers-"+kind)
      if(!button || !button.enabled)return false
      preferencesInput.mouseClick(button,button.width/2,button.height/2,Qt.LeftButton);return true
    }
    function trackerEditorState():string {
      var editor=dashboard.item ? root.find(dashboard.item.torrents,"tracker-editor") : null,input=editor ? root.find(editor,"tracker-url-input") : null
      return JSON.stringify(editor ? {mode:editor.mode,cancelFocused:editor.cancelFocused,masked:input ? input.password : false,display:input ? input.displayText : "",length:input ? input.text.length : 0} : {})
    }
    function clickDetailTab(tab:string):bool {
      if(Quickshell.env("FBT_TEST_HOOKS")!=="1" || !dashboard.item)return false
      var button=root.find(dashboard.item.torrents,"details-tab-"+tab);if(!button)return false
      preferencesInput.mouseClick(button,button.width/2,button.height/2,Qt.LeftButton);return true
    }
    function peerCountCell():string {
      if(Quickshell.env("FBT_TEST_HOOKS")!=="1")return "refused"
      var cell=root.find(dashboard.item ? dashboard.item.torrents : null,"connected-peer-count");return cell ? cell.renderedText : ""
    }
    function peerRows():string {
      if(Quickshell.env("FBT_TEST_HOOKS")!=="1")return "[]"
      var list=root.find(dashboard.item ? dashboard.item.torrents : null,"peer-list");return JSON.stringify(list ? list.renderedRows() : [])
    }
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
