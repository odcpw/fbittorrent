import QtQuick
import Quickshell
import Quickshell.Io
import QtTest
import qs.Commons as Commons
import "Plugin"
import "Plugin/Geometry.js" as Geometry
import "Plugin/BarState.js" as BarState
import "Reference"

ShellRoot {
  id:root
  property var captures: ({})
  property var facade: QtObject {
    function serviceFor(id) {return service}
    function summon(id,args) {console.log("FBTVISUAL-SUMMON "+id+" "+args);return false}
  }
  Service {id:service;shell:root.facade}
  QtObject {
    id:fixtureBar
    property var shell:root.facade
    property color foreground:Commons.Color.foreground
    property color barForeground:foreground
    property color urgent:Commons.Color.urgent
    property color background:Commons.Color.background
    property string fontFamily:Quickshell.env("FBT_FONT_FAMILY")
    property string position:"top"
    property bool vertical:false
    property bool foregroundAnimationEnabled:false
    property int barSize:32
    property var activePopout:null
    property var clickTargets:[]
    function requestPopout(owner) {activePopout=owner;return true}
    function releasePopout(owner) {if(activePopout===owner)activePopout=null}
    function registerClickTarget(item) {}
    function unregisterClickTarget(item) {}
    function targetBelongsToWindow(item,window) {return false}
    function hideTooltip(item) {}
    function showTooltip(item,text) {}
    function run(command) {console.log("FBTVISUAL-BLOCKED "+command)}
  }
  PanelWindow {
    visible:false
    screen:Quickshell.screens[0]
    implicitWidth:64;implicitHeight:32
    BarWidget {id:widget;bar:fixtureBar;shell:root.facade}
    NetworkWidget {id:reference;bar:fixtureBar;x:32}
  }
  function referenceItem() {
    for(var i=0;i<reference.data.length;i++) {
      var item=reference.data[i]
      if (item.loadedContent!==undefined) {item.ensureContent();return item.loadedContent}
    }
    return null
  }
  TestCase {id:input;name:"PopupPointer";when:false}
  function find(item,name){if(!item)return null;if(item.objectName===name)return item;for(var i=0;i<item.children.length;i++){var value=find(item.children[i],name);if(value)return value}return null}
  IpcHandler {
    target:"fbt-visual"
    function confirmed(hash:string):string{return JSON.stringify(service.store.torrents[hash] || null)}
    function peerLabels():string {
      if(Quickshell.env("FBT_TEST_HOOKS")!=="1")return "[]"
      var labels=[]
      function visit(item){if(!item)return;if(item.objectName==="popup-row-meta")labels.push(item.text);for(var i=0;i<item.children.length;i++)visit(item.children[i])}
      visit(widget.popupItem());return JSON.stringify(labels)
    }
    function offer():string {return JSON.stringify(service.clipboardOffer)}
    function clickPower():bool {if(Quickshell.env("FBT_TEST_HOOKS")!=="1")return false;var button=root.find(widget.popupItem(),"popup-power");if(!button)return false;input.mouseClick(button,button.width/2,button.height/2,Qt.LeftButton);return true}
    function clickClipboard(action:string):bool {if(Quickshell.env("FBT_TEST_HOOKS")!=="1" || ["add","dismiss"].indexOf(action)<0)return false;var button=root.find(widget.popupItem(),"clipboard-"+action);if(!button)return false;input.mouseClick(button,button.width/2,button.height/2,Qt.LeftButton);return true}
    function clickTurtle():bool {if(Quickshell.env("FBT_TEST_HOOKS")!=="1")return false;var button=root.find(widget.popupItem(),"popup-turtle");if(!button)return false;input.mouseClick(button,button.width/2,button.height/2,Qt.LeftButton);return true}
    function groups():string {return JSON.stringify(widget.popupItem() ? widget.popupItem().sections.map(function(g){return {key:g.key,count:g.rows.length}}) : [])}
    function cursor():string {var c=widget.popupItem() ? widget.popupItem().cursor() : {};c.opened=widget.opened;c.visible=widget.visible;return JSON.stringify(c)}
    function expand(group:string):bool {if(Quickshell.env("FBT_TEST_HOOKS")!=="1" || !widget.popupItem())return false;var item=widget.popupItem();for(var i=0;i<item.sections.length;i++)if(item.sections[i].key===group){if(!item.sections[i].open)item.toggleGroup(item.sections[i]);return true}return false}
    function state():string {return JSON.stringify({idle:widget.popupItem() ? widget.popupItem().presentation.idle : "",banner:widget.popupItem() ? widget.popupItem().presentation.banner : "",turtle:service.turtleOn,bar:BarState.describe(service),popupTurtle:widget.popupItem() ? widget.popupItem().turtleOn : false,online:service.online,rows:service.torrents.length,error:service.lastError,action:service.actionStatus,opened:widget.opened,mode:service.client ? service.client.mode : "",geometry:widget.geometry()})}
    function useRuntimeDir(path:string):string {return service.useRuntimeDir(path)}
    function open():void {reference.close();widget.open()}
    function close():void {reference.close();widget.close()}
    function referenceOpen():void {widget.close();reference.open()}
    function captureStatus(path:string):string {return root.captures[path] ? JSON.stringify(root.captures[path]) : "pending"}
    function capture(path:string,which:string):string {
      var item=which==="reference" ? root.referenceItem() : widget.popupItem()
      if(!item || item.width<=0 || item.height<=0)return "not ready"
      item=Geometry.card(item)
      if(item.opacity<0.999)return "not ready"
      root.captures[path]=null
      var info=which==="reference" ? Geometry.describe(item,Commons.Style.font.title) : JSON.parse(state()).geometry
      item.grabToImage(function(result){root.captures[path]={path:path,saved:result.saveToFile(path),geometry:info};console.log("FBTVISUAL-CAPTURE "+JSON.stringify(root.captures[path]))})
      return JSON.stringify(info)
    }
  }
}
