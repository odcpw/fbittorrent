import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import qs.Ui as Ui
import qs.Commons as Commons

ColumnLayout {
  id:root
  required property var flow
  property int revision:0
  property int cursor:0
  readonly property bool editing:urlInput.activeFocus
  signal closeRequested()
  function cycleFocus(delta){
    var items=[providerList,curated,urlInput,installButton,closeButton],index=items.findIndex(function(item){return item.activeFocus})
    items[(index+delta+items.length)%items.length].forceActiveFocus()
  }
  Component.onCompleted:providerList.forceActiveFocus()
  function key(event){
    if(event.key===Qt.Key_Tab || event.key===Qt.Key_Backtab){cycleFocus(event.key===Qt.Key_Backtab ? -1 : 1);return true}
    if(event.key===Qt.Key_Escape){closeRequested();return true}
    if(urlInput.activeFocus)return false
    if(event.text==="j" || event.key===Qt.Key_Down){providerList.forceActiveFocus();cursor=Math.min(flow.providers.length-1,cursor+1);return true}
    if(event.text==="k" || event.key===Qt.Key_Up){providerList.forceActiveFocus();cursor=Math.max(0,cursor-1);return true}
    if(event.key===Qt.Key_Space || event.key===Qt.Key_Return){if(flow.providers[cursor])flow.providerToggle(flow.providers[cursor]);return true}
    return false
  }
  spacing:Commons.Style.spacing.sm
  Text {text:"Providers";textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.title}
  ListView {
    id:providerList
    Layout.fillWidth:true;Layout.fillHeight:true;clip:true;keyNavigationEnabled:false
    model:root.revision>=0 ? root.flow.providers : []
    currentIndex:root.cursor
    delegate:Rectangle {
      id:provider
      required property var modelData
      required property int index
      width:ListView.view.width;height:Commons.Style.spacing.controlHeight
      color:index===root.cursor ? Commons.Style.selectedFill : Commons.Color.background
      RowLayout {
        anchors.fill:parent;spacing:Commons.Style.spacing.sm
        Text {Layout.fillWidth:true;text:(provider.modelData.full_name || provider.modelData.name)+" · "+provider.modelData.kind;textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
        Ui.ToggleSwitch {checked:provider.modelData.enabled;busy:root.revision>=0 && !!root.flow.providerBusy[provider.modelData.name];onToggled:root.flow.providerToggle(provider.modelData)}
      }
      TapHandler {onTapped:root.cursor=provider.index}
    }
  }
  Ui.Button {id:curated;focusable:true;text:"Install curated providers";onClicked:root.flow.install(null)}
  Ui.TextField {id:urlInput;Layout.fillWidth:true;maximumLength:4096;placeholderText:"HTTPS provider URLs, separated by spaces";onAccepted:installSources()}
  function installSources(){var urls=urlInput.text.trim().split(/\s+/).filter(function(s){return !!s});if(urls.length && urls.length<=16 && urls.every(function(s){return /^https:\/\//.test(s)}))root.flow.install(urls);else {root.flow.message="Enter 1 to 16 HTTPS provider URLs";root.flow.providersRefresh()}}
  RowLayout {Ui.Button {id:installButton;focusable:true;text:"Install URLs";onClicked:root.installSources()} Ui.Button {id:closeButton;focusable:true;text:"Close (Esc)";onClicked:root.closeRequested()}}
  Text {Layout.fillWidth:true;wrapMode:Text.Wrap;maximumLineCount:6;elide:Text.ElideRight;text:root.revision>=0 ? root.flow.message : "";textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
}
