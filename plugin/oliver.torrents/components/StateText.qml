import QtQuick
import qs.Commons as Commons
import "../Model.js" as Model

Text {
  required property var torrent
  property color foreground:Commons.Color.foreground
  text:Model.stateLabel(torrent)
  textFormat:Text.PlainText
  color:torrent.state==="error" ? Commons.Color.urgent : foreground
  font.family:Commons.Style.font.family
  font.pixelSize:Commons.Style.font.subtitle
  elide:Text.ElideRight
}
