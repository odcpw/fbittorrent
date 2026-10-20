import QtQuick
import QtQuick.Layouts
import qs.Ui as Ui
import qs.Commons as Commons
import "../Model.js" as Model

// One explicit choice for several selected results: every file, the default
// download folder and no upload. Single results keep their full review sheet.
ColumnLayout {
  id:root
  required property var flow
  required property var service
  property int revision:0
  readonly property bool editing:false
  readonly property var rows:revision>=0 ? flow.selectedRows() : []
  readonly property real bytes:rows.reduce(function(sum,row){return sum+Number(row.size || 0)},0)
  readonly property var settings:service.store.settings.settings || service.store.settings || {}
  readonly property string folder:String(settings.save_path || "")
  property int selected:1
  signal closeRequested()
  Component.onCompleted:confirm.forceActiveFocus()
  function move(dx){selected=Math.max(0,Math.min(1,selected+dx));(selected ? confirm : cancel).forceActiveFocus()}
  function key(event){
    if(event.key===Qt.Key_Escape || event.text==="n"){closeRequested();return true}
    if(event.text==="y"){flow.batchDownload();return true}
    if(event.key===Qt.Key_Return || event.key===Qt.Key_Enter){if(selected)flow.batchDownload();else closeRequested();return true}
    if(event.key===Qt.Key_Left || event.text==="h" || event.key===Qt.Key_Backtab){move(-1);return true}
    if(event.key===Qt.Key_Right || event.text==="l" || event.key===Qt.Key_Tab){move(1);return true}
    return true
  }
  spacing:Commons.Style.spacing.sm
  Text {text:"Download "+root.rows.length+(root.rows.length===1 ? " result" : " results");textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.title}
  Text {
    Layout.fillWidth:true;wrapMode:Text.Wrap;textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body
    text:"All files · "+Model.size(root.bytes,false)+" · no upload\nSave to "+(root.folder || "the default download folder")
  }
  ListView {
    id:list
    Layout.fillWidth:true;Layout.fillHeight:true;clip:true;interactive:true;keyNavigationEnabled:false
    model:root.rows
    delegate:Text {
      required property var modelData
      width:ListView.view.width;elide:Text.ElideRight;textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body
      text:"✓ "+modelData.name+" · "+Model.size(modelData.size,true)
    }
  }
  Text {Layout.fillWidth:true;wrapMode:Text.Wrap;text:"To pick files or a different folder, add results one at a time with Enter.";textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.caption}
  RowLayout {
    spacing:Commons.Style.spacing.md
    Ui.Button {id:cancel;text:"Cancel (Esc)";focusable:true;bordered:true;onClicked:root.closeRequested()}
    Ui.Button {id:confirm;objectName:"search-batch-confirm";text:"Download "+root.rows.length+" (Enter)";focusable:true;bordered:true;enabled:root.rows.length>0;onClicked:root.flow.batchDownload()}
  }
}
