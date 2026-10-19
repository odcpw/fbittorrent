import QtQuick
import qs.Commons as Commons
import "../Model.js" as Model

Text {
  textFormat:Text.PlainText
  color:Model.dim(Commons.Color.foreground,Commons.Color.background)
  font.family:Commons.Style.font.family
  font.pixelSize:Commons.Style.font.caption
  elide:Text.ElideRight
}
