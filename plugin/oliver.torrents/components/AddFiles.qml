import QtQuick
import QtQuick.Layouts
import qs.Commons as Commons
import qs.Ui as Ui
import "../Model.js" as Model
ListView {
  id:root
  required property var flow
  required property var sheet
  model:sheet.files
  clip:true;reuseItems:true
  implicitHeight:Commons.Style.space(220)
  activeFocusOnTab:true
  currentIndex:0
  boundsBehavior:Flickable.StopAtBounds
  Keys.onPressed:function(event){
    if(event.key===Qt.Key_Space){var row=root.sheet.files[currentIndex];if(row)flow.toggle(row.index);event.accepted=true}
    else if(event.text==="j"){currentIndex=Math.min(count-1,currentIndex+1);event.accepted=true}
    else if(event.text==="k"){currentIndex=Math.max(0,currentIndex-1);event.accepted=true}
  }
  delegate:Rectangle {
    required property var modelData
    required property int index
    width:root.width;height:Commons.Style.spacing.controlHeight
    color:root.activeFocus && root.currentIndex===index ? Commons.Style.selectedFill : Commons.Color.popups.background
    RowLayout {
      anchors.fill:parent;spacing:Commons.Style.spacing.lg
      Ui.Button {text:root.sheet.included[modelData.index] ? "☑" : "☐";onClicked:{root.currentIndex=index;root.forceActiveFocus();root.flow.toggle(modelData.index)}}
      Text {Layout.fillWidth:true;textFormat:Text.PlainText;text:modelData.path;elide:Text.ElideMiddle;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body
        HoverHandler {id:hover}
        FullTextTip {visible:hover.hovered;text:parent.text}
      }
      Text {textFormat:Text.PlainText;text:Model.size(modelData.size,false);color:Model.dim(Commons.Color.foreground,Commons.Color.popups.background);font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.caption}
    }
  }
}
