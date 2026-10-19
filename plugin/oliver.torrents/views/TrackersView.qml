import QtQuick
import QtQuick.Layouts
import qs.Ui as Ui
import qs.Commons as Commons
FocusScope {
  id:root
  required property var service
  required property var flow
  objectName:"tracker-view"
  readonly property string topic:"torrent:"+flow.hash+".trackers"
  readonly property var rows:service.detailRevision>=0 ? Object.values(service.store.topics[topic] || {}).sort(function(a,b){return a.url.localeCompare(b.url)}) : []
  readonly property bool busy:service.detailRevision>=0 && flow.trackerBusy
  readonly property bool editable:service.online && !busy && service.client && service.client.caps.indexOf("tracker_edit")>=0
  readonly property bool editing:editor.opened
  property string selectedUrl:""
  onRowsChanged:{if(!rows.some(function(row){return row.url===selectedUrl}))selectedUrl=rows.length ? rows[0].url : ""}
  function renderedRows(){var result=[];for(var i=0;i<list.count;i++){var row=list.itemAtIndex(i);if(row)result.push(row.rendered())}return result}
  function key(event){
    if(editing)return editor.key(event)
    if(event.text==="e"){root.flow.reannounce();return true}
    if(event.text==="+" && editable){editor.show(true,"");return true}
    if(event.text==="x" && editable && selectedUrl){editor.show(false,selectedUrl);return true}
    if(event.key===Qt.Key_Up || event.key===Qt.Key_Down || event.text==="j" || event.text==="k"){
      var index=rows.findIndex(function(row){return row.url===selectedUrl}),delta=event.key===Qt.Key_Up || event.text==="k" ? -1 : 1
      index=Math.max(0,Math.min(rows.length-1,index+delta));if(rows[index]){selectedUrl=rows[index].url;list.positionViewAtIndex(index,ListView.Contain)}return true
    }
    return false
  }
  ColumnLayout {
  anchors.fill:parent;spacing:Commons.Style.spacing.sm
  RowLayout {
    Layout.fillWidth:true
    Ui.Button {objectName:"trackers-reannounce";text:root.busy ? "Reannouncing…" : "Reannounce (e)";enabled:root.service.online && !root.busy;onClicked:root.flow.reannounce()}
    Ui.Button {objectName:"trackers-add";text:"Add (+)";enabled:root.editable;onClicked:editor.show(true,"")}
    Ui.Button {objectName:"trackers-remove";text:"Remove (x)";enabled:root.editable && !!root.selectedUrl;onClicked:editor.show(false,root.selectedUrl)}
    Item {Layout.fillWidth:true}
  }
  ListView {
    id:list
    Layout.fillWidth:true;Layout.fillHeight:true;model:root.rows;clip:true;spacing:Commons.Style.spacing.lg
    delegate:TrackerRow {required property var modelData;tracker:modelData;width:ListView.view.width;opacity:root.selectedUrl===modelData.url ? 1 : .65;TapHandler {onTapped:root.selectedUrl=parent.modelData.url}}
    Text {anchors.centerIn:parent;width:parent.width;textFormat:Text.PlainText;wrapMode:Text.Wrap;text:root.service.store.topics[root.topic]===undefined ? "Loading trackers…" : "No trackers recorded.";visible:list.count===0;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
  }
  }
  TrackerEditor {id:editor;objectName:"tracker-editor";anchors.fill:parent;flow:root.flow}
}
