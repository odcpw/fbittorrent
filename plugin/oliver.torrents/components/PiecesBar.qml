import QtQuick
import qs.Commons as Commons
import "../PiecesModel.js" as PiecesModel
Column {
 id:root
 property var snapshot:null
 property var torrent:null
 readonly property var map:PiecesModel.decode(snapshot)
 readonly property var cells:PiecesModel.cells(map,Math.floor(width/Commons.Style.space(4)))
 readonly property var state:map ? {count:map.data.count,have:map.have,priority:map.priority,playable_end:map.data.playable_end,cells:cells.length} : null
 spacing:Commons.Style.spacing.sm
 Text {width:root.width;text:root.map ? root.map.have+" / "+root.map.data.count+" verified pieces" : PiecesModel.unavailable(root.torrent);textFormat:Text.PlainText;wrapMode:Text.Wrap;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
 Item {
  width:root.width;height:Commons.Style.space(28)
  Row {
   Repeater {
    model:root.cells
    PieceCell {}
   }
  }
 }
 Text {width:root.width;text:"▲ first/last priority · bright verified · dim missing · underline playable prefix";textFormat:Text.PlainText;wrapMode:Text.Wrap;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.caption}
}
