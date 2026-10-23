import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import qs.Ui as Ui
import qs.Commons as Commons
import "../Model.js" as Model
import "../components" as Components

ColumnLayout {
  id:root
  required property var service
  required property var flow
  required property var torrent
  property int revision:service.detailRevision
  readonly property var rows:revision>=0 ? flow.files : []
  readonly property bool editing:filterInput.activeFocus
  function focusFilter(){filterInput.forceActiveFocus()}
  function endFilter(){service.cancelFocusTarget(filterInput);filterInput.focus=false;root.forceActiveFocus()}
  function key(event){
    if(editing){if(event.key===Qt.Key_Escape || event.key===Qt.Key_Return){endFilter();return true}return false}
    if(event.text==="/"){focusFilter();return true}
    if(event.text==="j" || event.key===Qt.Key_Down){flow.move(1);return true}
    if(event.text==="k" || event.key===Qt.Key_Up){flow.move(-1);return true}
    if(event.key===Qt.Key_Space){flow.toggle();return true}
    if(event.text==="a" || event.text==="n"){flow.setFiles(event.text==="a",true);return true}
    if(event.text==="o"){flow.openFile();return true}
    if(event.text==="O"){service.openFolder(torrent.hash);return true}
    return false
  }
  spacing:Commons.Style.spacing.sm
  Ui.TextField {id:filterInput;Layout.fillWidth:true;text:root.revision>=0 ? root.flow.query : "";placeholderText:"/ filter files…";maximumLength:200;onTextEdited:root.flow.filter(text);Keys.onPressed:function(event){if(event.key===Qt.Key_Escape || event.key===Qt.Key_Return){root.endFilter();event.accepted=true}}}
  Components.KeyHint {Layout.fillWidth:true;text:root.torrent.state==="metadata" ? "resolving metadata…" : root.torrent.state==="checking" ? "Checking files; selection is locked" : "Space toggle · a include filtered · n skip filtered · o file · O folder"}
  ListView {
    id:list
    Layout.fillWidth:true;Layout.fillHeight:true;clip:true
    model:root.rows
    currentIndex:root.revision>=0 ? root.flow.cursor : 0
    onCurrentIndexChanged:positionViewAtIndex(currentIndex,ListView.Contain)
    delegate:Rectangle {
      id:row
      required property var modelData
      required property int index
      width:list.width;height:Math.max(Commons.Style.space(58),fileText.implicitHeight+Commons.Style.spacing.sm*2)
      color:index===list.currentIndex ? Commons.Util.alpha(Commons.Color.accent,0.15) : Commons.Color.background
      RowLayout {
        anchors.fill:parent;anchors.margins:Commons.Style.spacing.xs;spacing:Commons.Style.spacing.sm
        Text {text:row.modelData.included ? "☑︎" : "☐︎";textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
        ColumnLayout {
          Layout.fillWidth:true;spacing:Commons.Style.spacing.xxs
          Text {id:fileText;Layout.fillWidth:true;text:row.modelData.path;textFormat:Text.PlainText;wrapMode:Text.Wrap;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
          Text {Layout.fillWidth:true;text:Model.size(row.modelData.size,false)+" · "+Model.percent(row.modelData.progress,row.modelData.progress===1)+(row.modelData.ready===true ? " · ● ready" : "");textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.caption}
        }
      }
      MouseArea {anchors.fill:parent;enabled:root.torrent.state!=="checking";onClicked:{root.flow.cursor=row.index;root.flow.toggle()} onDoubleClicked:root.service.openTarget(root.torrent.hash,row.modelData.index)}
    }
    Text {anchors.centerIn:parent;visible:list.count===0;text:root.torrent.state==="metadata" ? "resolving metadata…" : root.flow.loading ? "Loading files…" : "No files match";textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
  }
}
