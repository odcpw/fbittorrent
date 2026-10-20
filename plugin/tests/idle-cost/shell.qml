import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons as Commons
import qs.Ui as Ui
import "Plugin"
import "Plugin/Glyphs.js" as Glyphs
import "Host"
import "HostBar" as HostBar

ShellRoot {
  id:root
  readonly property bool enabled:Quickshell.env("FBT_IDLE_PLUGIN")==="1"
  readonly property bool daemonUp:Quickshell.env("FBT_IDLE_SCENARIO")!=="down"
  readonly property var service:serviceLoader.item
  readonly property string widgetId:enabled ? "oliver.torrents" : "fbt.idle-baseline"
  readonly property var widget:bar.moduleWidgets(widgetId)[0] || null
  property int barFrames:0
  property int popupFrames:0
  property int baselineWidth:380
  property int baselineHeight:300
  Loader {id:serviceLoader;active:root.enabled;sourceComponent:Component {Service {shell:facade;settings:({binary:Quickshell.env("FBT_TEST_BINARY")})}}}
  BarWidgetRegistry {id:registry}
  Component {id:pluginComponent;BarWidget {}}
  Component {
    id:baselineComponent
    Ui.Panel {
      id:blank
      moduleName:"fbt.idle-baseline";manageIpc:false
      implicitWidth:button.implicitWidth;implicitHeight:button.implicitHeight
      visible:root.daemonUp || opened
      function popupItem(){return panel.loadedContent}
      Ui.BarIconButton {id:button;anchors.fill:parent;bar:blank.bar;text:Glyphs.download}
      Ui.KeyboardPanel {
        id:panel;anchorItem:button;owner:blank;bar:blank.bar;open:blank.opened
        contentWidth:panel.fittedContentWidth(root.baselineWidth)
        contentHeight:panel.fittedContentHeight(root.baselineHeight,560)
        content:Component {Item {implicitWidth:root.baselineWidth;implicitHeight:root.baselineHeight}}
      }
    }
  }
  PluginShellApi {id:facade;pluginId:"oliver.torrents";_serviceLookup:function(id){return root.service};_summon:function(){return false}}
  QtObject {id:host;function pluginShellForId(id){return facade}}
  HostBar.Bar {
    id:bar;shell:host;barWidgetRegistry:registry
    barConfig:({position:"top",background:"opaque",layout:{left:[],centerBegin:[],center:[],centerEnd:[],right:[{id:root.widgetId}]}})
  }
  Connections {target:root.widget ? root.widget.Window.window : null;function onFrameSwapped(){root.barFrames++}}
  Connections {target:root.widget && root.widget.popupItem() ? root.widget.popupItem().Window.window : null;function onFrameSwapped(){root.popupFrames++}}
  IpcHandler {
    target:"fbt-dashboard"
    function ready():bool{return !!root.widget && (!root.enabled || (!!root.service && root.service.online===root.daemonUp))}
    function popup(open:bool):bool{if(!root.widget)return false;if(open)root.widget.open();else root.widget.close();return true}
    function size(width:int,height:int):bool{root.baselineWidth=width;root.baselineHeight=height;return true}
    function state():string {
      var s=root.service,p=root.widget && root.widget.popupItem()
      return JSON.stringify({plugin:root.enabled,online:s ? s.online : false,opened:!!root.widget && root.widget.opened,
        widgetVisible:!!root.widget && root.widget.visible,barFrames:root.barFrames,popupFrames:root.popupFrames,
        width:p ? p.width : 0,height:p ? p.height : 0,
        counters:s ? s.perfCounters : {},pending:s && s.client ? Object.keys(s.client.pending).length : 0,
        rows:s ? s.torrents.length : 0,appearance:s ? s.status : null})
    }
  }
  Component.onCompleted:registry.register(root.widgetId,root.enabled ? pluginComponent : baselineComponent,{firstParty:false})
}
