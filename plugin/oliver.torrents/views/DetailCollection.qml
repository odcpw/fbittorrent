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
  delegate:Text {
    required property var modelData
    width:ListView.view.width;textFormat:Text.PlainText;wrapMode:Text.Wrap
    text:root.flow.tab==="trackers" ? modelData.url+"\n"+modelData.status+(modelData.last_error ? " · "+modelData.last_error : "") : root.flow.tab==="peers" ? (modelData.address || modelData.ip || "")+" · "+Model.rate(modelData.dl_bps,false) : "Piece "+modelData.index+" · "+(modelData.state || "")
    color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body
  }
  Text {anchors.centerIn:parent;width:parent.width;wrapMode:Text.Wrap;visible:root.count===0;text:root.service.lastError || "No "+root.flow.tab+" reported";textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
}
