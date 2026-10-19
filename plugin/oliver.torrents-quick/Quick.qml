import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Wayland
import Quickshell.Hyprland
import qs.Ui as Ui
import qs.Commons as Commons
import "QuickFlow.js" as QuickFlow

Item {
  id:root
  property var shell:null
  property var manifest:({id:"oliver.torrents-quick"})
  readonly property var service:shell ? shell.serviceFor("oliver.torrents") : null
  property var flow:null
  property int revision:0
  property bool opened:false
  readonly property string queryText:input.text
  readonly property bool inputFocused:input.activeFocus
  property var targetScreen:null
  readonly property var viewModel:revision>=0 && flow ? flow.snapshot() : ({rows:[],phase:"idle",cursor:0,message:""})
  Commons.Translations {id:strings;directory:Qt.resolvedUrl("i18n")}
  function ensureFlow(){if(!flow && service)flow=QuickFlow.create(service,function(){root.revision++});return !!flow}
  function open(payloadJson){
    if(!ensureFlow())return false
    var payload={};try{payload=JSON.parse(payloadJson || "{}")}catch(error){return false}
    if(!payload || typeof payload!=="object" || Array.isArray(payload))return false
    var text=typeof payload.query==="string" ? payload.query.slice(0,65536) : ""
    targetScreen=Quickshell.screens[0]
    var monitor=Hyprland.focusedMonitor
    for(var i=0;monitor && i<Quickshell.screens.length;i++)if(Quickshell.screens[i].name===monitor.name)targetScreen=Quickshell.screens[i]
    flow.open();input.text=text;opened=true
    if(!service.online)service.startDaemon()
    if(text && !flow.looksSource(text))debounce.restart()
    Qt.callLater(function(){input.forceActiveFocus()});return true
  }
  function close(){debounce.stop();if(flow)flow.close();opened=false}
  function dismiss(){close();if(shell && typeof shell.hide==="function")shell.hide("oliver.torrents-quick")}
  function toggle(){if(opened)dismiss();else open("{}")}
  function activate(){
    debounce.stop();if(!ensureFlow())return false
    if(!service.online){service.startDaemon();flow.error(strings.t("starting"));return false}
    var source=flow.source(input.text)
    if(source)return flow.add(source,dismiss)
    if(flow.looksSource(input.text)){flow.error(strings.t("invalidSource"));return false}
    if(input.text.trim()!==flow.query || !flow.current())return flow.search(input.text)
    if(!shell || typeof shell.summon!=="function"){flow.error(strings.t("dashboardUnavailable"));return false}
    // Pass the selected full result; cancelling our session cannot invalidate
    // the queued grab while the dashboard's shared AddFlow resolves metadata.
    if(!service.beginSearchAdd(flow.current(),0)){flow.error(strings.t("finishReview"));return false}
    if(!shell.summon("oliver.torrents",JSON.stringify({view:"torrents"}))){flow.error(strings.t("dashboardUnavailable"));return false}
    dismiss();return true
  }
  function key(event){
    if(event.key===Qt.Key_Escape){dismiss();return true}
    if(event.key===Qt.Key_Down){flow.move(1);return true}
    if(event.key===Qt.Key_Up){flow.move(-1);return true}
    if(event.key===Qt.Key_Return || event.key===Qt.Key_Enter){activate();return true}
    return false
  }
  Component.onDestruction:close()
  Connections {target:root.service
    function onApiFrame(frame,request){if(root.flow)root.flow.frame(frame)}
    function onOnlineChanged(){if(root.opened && root.service.online && input.text && !root.flow.looksSource(input.text))debounce.restart()}
  }
  Timer {id:debounce;interval:350;onTriggered:if(root.opened && root.service.online)root.flow.search(input.text)}
  PanelWindow {
    id:window;visible:root.opened;screen:root.targetScreen
    anchors {top:true;bottom:true;left:true;right:true}
    exclusionMode:ExclusionMode.Ignore;color:"transparent"
    WlrLayershell.layer:WlrLayer.Overlay;WlrLayershell.keyboardFocus:WlrKeyboardFocus.Exclusive
    WlrLayershell.namespace:"oliver.torrents-quick"
    onVisibleChanged:if(visible)Qt.callLater(function(){input.forceActiveFocus()})
    Rectangle {anchors.fill:parent;color:Commons.Util.alpha(Commons.Color.background,0.65)}
    MouseArea {anchors.fill:parent;onClicked:root.dismiss()}
    Ui.BorderSurface {
      width:Math.min(parent.width-Commons.Style.space(32),Commons.Style.space(640))
      height:Math.min(parent.height-Commons.Style.space(32),content.implicitHeight+Commons.Style.spacing.panelPadding*2)
      anchors.centerIn:parent;color:Commons.Color.menu.background
      borderSpec:Commons.Border.surfaceSpec("menu","border",Commons.Color.menu.border,Math.max(1,Commons.Style.space(2)))
      padding:Commons.Style.spacing.panelPadding
      MouseArea {anchors.fill:parent;onClicked:{}}
      ColumnLayout {
        id:content;anchors.fill:parent;spacing:Commons.Style.spacing.sm
        Text {Layout.fillWidth:true;text:strings.t("title");textFormat:Text.PlainText;color:Commons.Color.menu.text;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.title}
        Ui.FilterInput {
          id:input;objectName:"quickInput";enabled:root.viewModel.phase!=="adding";Layout.fillWidth:true;Layout.preferredHeight:Commons.Style.space(36);maximumLength:65536
          color:Commons.Color.menu.text;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body
          onTextEdited:{if(root.flow){root.flow.edited(text);if(!root.flow.looksSource(text))debounce.restart();else debounce.stop()}}
          Keys.onPressed:function(event){if(root.key(event))event.accepted=true}
        }
        Text {Layout.fillWidth:true;visible:!input.text;text:strings.t("hint");textFormat:Text.PlainText;color:Commons.Color.menu.text;font.pixelSize:Commons.Style.font.caption;wrapMode:Text.Wrap}
        ListView {
          id:results;Layout.fillWidth:true;Layout.preferredHeight:root.viewModel.rows.length*Commons.Style.space(44);Layout.maximumHeight:Math.max(0,window.height-Commons.Style.space(220))
          clip:true;model:root.viewModel.rows;currentIndex:root.viewModel.cursor
          onCurrentIndexChanged:positionViewAtIndex(currentIndex,ListView.Contain)
          delegate:Rectangle {
            required property var modelData
            required property int index
            width:results.width;height:Commons.Style.space(44)
            color:index===root.viewModel.cursor ? Commons.Color.menu.selectedBackground : "transparent"
            Column {anchors.fill:parent;anchors.margins:Commons.Style.spacing.xs
              Text {width:parent.width;text:modelData.name;textFormat:Text.PlainText;elide:Text.ElideRight;color:index===root.viewModel.cursor ? Commons.Color.menu.selectedText : Commons.Color.menu.text;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
              Text {width:parent.width;text:(modelData.providers || []).join(", ");textFormat:Text.PlainText;elide:Text.ElideRight;color:Commons.Color.menu.text;font.pixelSize:Commons.Style.font.caption}
            }
            MouseArea {anchors.fill:parent;onClicked:{root.flow.cursor=index;root.activate()}}
          }
        }
        Text {Layout.fillWidth:true;visible:!!root.viewModel.message || root.viewModel.phase==="searching" || root.viewModel.phase==="adding";text:root.viewModel.message || strings.t(root.viewModel.phase);textFormat:Text.PlainText;wrapMode:Text.Wrap;color:root.viewModel.phase==="error" ? Commons.Color.urgent : Commons.Color.menu.text;font.pixelSize:Commons.Style.font.caption}
        Text {Layout.fillWidth:true;text:strings.t("keys");textFormat:Text.PlainText;color:Commons.Color.menu.text;font.pixelSize:Commons.Style.font.caption}
      }
    }
  }
}
