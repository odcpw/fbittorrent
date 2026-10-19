import QtQuick
import qs.Commons as Commons

Item {
  required property var modelData
  width:Commons.Style.space(4)
  height:Commons.Style.space(28)
  Rectangle {
    objectName:"piece-completion"
    width:parent.width;height:Commons.Style.space(12)
    color:parent.modelData.have ? Commons.Color.foreground : Commons.Util.alpha(Commons.Color.foreground,0.22)
  }
  Text {
    objectName:"piece-priority"
    anchors.top:parent.top;anchors.topMargin:Commons.Style.space(12)
    text:parent.modelData.priority ? "▲" : ""
    textFormat:Text.PlainText;color:Commons.Color.accent
    font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.caption
  }
  Rectangle {
    anchors.bottom:parent.bottom;width:parent.width;height:Commons.Style.space(2)
    visible:parent.modelData.prefix;color:Commons.Color.foreground
  }
}
