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
  signal closeRequested()
  Component.onCompleted:sourceList.forceActiveFocus()
  function key(event){
    if(event.key===Qt.Key_Tab || event.key===Qt.Key_Backtab){if(closeButton.activeFocus)sourceList.forceActiveFocus();else closeButton.forceActiveFocus();return true}
    if(event.key===Qt.Key_Escape){closeRequested();return true}
    if(event.text==="j" || event.key===Qt.Key_Down){sourceList.forceActiveFocus();cursor=Math.min(flow.sources.length-1,cursor+1);return true}
    if(event.text==="k" || event.key===Qt.Key_Up){sourceList.forceActiveFocus();cursor=Math.max(0,cursor-1);return true}
    if(event.key===Qt.Key_Return){if(flow.sources[cursor])flow.browse(flow.sources[cursor]);return true}
    return false
  }
  spacing:Commons.Style.spacing.sm
  Text {text:"Browse lists";textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.title}
  ListView {
    id:sourceList
    Layout.fillWidth:true;Layout.fillHeight:true;clip:true;keyNavigationEnabled:false
    model:root.revision>=0 ? root.flow.sources : []
    currentIndex:root.cursor
    delegate:Ui.Button {
      required property var modelData
      required property int index
      focusable:true;width:ListView.view.width;text:modelData.label+" · "+modelData.provider+(modelData.in_app ? "" : " ↗ browser")
      hasCursor:index===root.cursor
      onClicked:{root.cursor=index;root.flow.browse(modelData)}
    }
  }
  Ui.Button {id:closeButton;focusable:true;text:"Close (Esc)";onClicked:root.closeRequested()}
}
