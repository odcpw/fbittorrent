import QtQuick
import qs.Commons as Commons
import "../PiecesModel.js" as PiecesModel
Column {
 id:root
 property var snapshot:null
 readonly property var map:PiecesModel.decode(snapshot)
 readonly property var cells:PiecesModel.cells(map,Math.floor(width/Commons.Style.space(4)))
 readonly property var state:map ? {count:map.data.count,have:map.have,priority:map.priority,playable_end:map.data.playable_end,cells:cells.length} : null
 spacing:Commons.Style.spacing.sm
 Text {width:root.width;text:root.map ? root.map.have+" / "+root.map.data.count+" verified pieces" : "Verified pieces unavailable until checking completes";textFormat:Text.PlainText;wrapMode:Text.Wrap;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
 Item {
  width:root.width;height:Commons.Style.space(22)
  Row {
   Repeater {
    model:root.cells
    Item {
     required property var modelData
     width:Commons.Style.space(4);height:Commons.Style.space(22)
     Rectangle {width:Commons.Style.space(4);height:Commons.Style.space(12);color:parent.modelData.priority ? Commons.Color.accent : parent.modelData.have ? Commons.Color.foreground : Commons.Util.alpha(Commons.Color.foreground,0.22)}
     Text {anchors.top:parent.top;anchors.topMargin:Commons.Style.space(10);text:parent.modelData.priority ? "▲" : "";textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.caption}
     Rectangle {anchors.bottom:parent.bottom;width:Commons.Style.space(4);height:Commons.Style.space(2);visible:parent.modelData.prefix;color:Commons.Color.foreground}
    }
   }
  }
 }
 Text {width:root.width;text:"▲ first/last priority · bright verified · dim missing · underline playable prefix";textFormat:Text.PlainText;wrapMode:Text.Wrap;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.caption}
}
