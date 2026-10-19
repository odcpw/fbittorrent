import QtQuick
import QtQuick.Controls
import qs.Commons as Commons
import "../Model.js" as Model

ListView {
  id:root
  required property var service
  required property var flow
  property string topic:"torrent:"+flow.hash+"."+flow.tab
  readonly property var rows:service.detailRevision>=0 ? Object.values(service.store.topics[topic] || {}) : []
  model:rows;clip:true;spacing:Commons.Style.spacing.sm
  objectName:root.flow.tab==="peers" ? "peer-list" : "detail-collection"
  function renderedRows(){var result=[];for(var i=0;i<count;i++){var row=itemAtIndex(i);if(row && root.flow.tab==="peers")result.push(row.rendered())}return result}
  delegate:Item {
    id:row
    required property var modelData
    width:ListView.view.width
    height:root.flow.tab==="peers" ? peer.implicitHeight : generic.implicitHeight
    function rendered(){return peer.rendered()}
    PeerRow {id:peer;width:parent.width;visible:root.flow.tab==="peers";peer:row.modelData}
    Text {
      id:generic
      width:parent.width;visible:root.flow.tab!=="peers";textFormat:Text.PlainText;wrapMode:Text.Wrap
      text:"Piece "+row.modelData.index+" · "+(row.modelData.state || "")
      color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body
    }
  }
  Text {anchors.centerIn:parent;width:parent.width;wrapMode:Text.Wrap;visible:root.count===0;text:service.store.topics[topic]===undefined ? "Loading "+root.flow.tab+"…" : "No "+root.flow.tab+" reported";textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
}
