import QtQuick
import QtQuick.Layouts
import qs.Ui as Ui
import qs.Commons as Commons
ColumnLayout {
  id:root
  required property var service
  required property var flow
  objectName:"tracker-view"
  readonly property string topic:"torrent:"+flow.hash+".trackers"
  readonly property var rows:service.detailRevision>=0 ? Object.values(service.store.topics[topic] || {}).sort(function(a,b){return a.url.localeCompare(b.url)}) : []
  readonly property bool busy:service.detailRevision>=0 && flow.trackerBusy
  spacing:Commons.Style.spacing.sm
  function renderedRows(){var result=[];for(var i=0;i<list.count;i++){var row=list.itemAtIndex(i);if(row)result.push(row.rendered())}return result}
  function key(event){if(event.text==="e"){root.flow.reannounce();return true}return false}
  RowLayout {
    Layout.fillWidth:true
    Ui.Button {objectName:"trackers-reannounce";text:root.busy ? "Reannouncing…" : "Reannounce (e)";enabled:root.service.online && !root.busy;onClicked:root.flow.reannounce()}
    Item {Layout.fillWidth:true}
  }
  ListView {
    id:list
    Layout.fillWidth:true;Layout.fillHeight:true;model:root.rows;clip:true;spacing:Commons.Style.spacing.lg
    delegate:TrackerRow {required property var modelData;tracker:modelData;width:ListView.view.width}
    Text {anchors.centerIn:parent;width:parent.width;textFormat:Text.PlainText;wrapMode:Text.Wrap;text:root.service.store.topics[root.topic]===undefined ? "Loading trackers…" : "No trackers recorded.";visible:list.count===0;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
  }
}
