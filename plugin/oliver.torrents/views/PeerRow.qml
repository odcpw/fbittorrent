import QtQuick
import QtQuick.Layouts
import qs.Commons as Commons
import "../Model.js" as Model

ColumnLayout {
  id:root
  required property var peer
  spacing:Commons.Style.spacing.xs
  function rendered(){return [address.text,connection.text,rates.text,counters.text].join("\n")}
  Text {
    id:address
    Layout.fillWidth:true;textFormat:Text.PlainText;wrapMode:Text.WrapAnywhere
    text:(root.peer.addr || "—")+" · "+(root.peer.client || "—")
    color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body
  }
  Text {
    id:connection
    Layout.fillWidth:true;textFormat:Text.PlainText;wrapMode:Text.Wrap
    text:(root.peer.incoming===true ? "IN" : root.peer.incoming===false ? "OUT" : "—")+" · "+(root.peer.conn==="tcp_mse" ? "TCP / MSE" : root.peer.conn==="tcp" ? "TCP" : "—")+" · "+(root.peer.state || "—")+" · seed "+(root.peer.seed===true ? "yes" : root.peer.seed===false ? "no" : "—")
    color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.bodySmall
  }
  Text {
    id:rates
    Layout.fillWidth:true;textFormat:Text.PlainText;wrapMode:Text.Wrap
    text:"↓ "+Model.rate(root.peer.dl_bps,false)+" · ↑ "+Model.rate(root.peer.ul_bps,false)
    color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body
  }
  Text {
    id:counters
    Layout.fillWidth:true;textFormat:Text.PlainText;wrapMode:Text.Wrap
    text:"Downloaded "+Model.size(root.peer.downloaded,false)+" · uploaded "+Model.size(root.peer.uploaded,false)
    color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.bodySmall
  }
}
