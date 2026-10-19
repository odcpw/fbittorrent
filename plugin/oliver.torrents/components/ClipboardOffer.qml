import QtQuick
import QtQuick.Layouts
import qs.Commons as Commons
import qs.Ui as Ui

Rectangle {
  id:root
  required property var service
  readonly property var offer:service ? service.clipboardOffer : null
  visible:!!offer
  implicitHeight:visible ? content.implicitHeight+Commons.Style.spacing.xxl : 0
  height:implicitHeight
  color:Commons.Util.alpha(Commons.Color.foreground,0.10)
  border.color:Commons.Util.alpha(Commons.Color.foreground,0.35)
  border.width:Commons.Style.space(1)
  radius:Commons.Style.cornerRadius
  ColumnLayout {
    id:content
    anchors.left:parent.left;anchors.right:parent.right;anchors.verticalCenter:parent.verticalCenter
    anchors.margins:Commons.Style.spacing.md
    spacing:Commons.Style.spacing.md
    Text {
      Layout.fillWidth:true;textFormat:Text.PlainText
      text:root.offer ? "Add '"+(root.offer.name || "magnet link")+"'?" : ""
      color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body
      wrapMode:Text.Wrap
    }
    RowLayout {
      spacing:Commons.Style.spacing.xxl
      Ui.Button {objectName:"clipboard-add";text:"Add (y)";focusable:true;bordered:true;onClicked:root.service.acceptOffer()}
      Ui.Button {objectName:"clipboard-dismiss";text:"Dismiss (Esc)";focusable:true;bordered:true;onClicked:root.service.cancelOffer()}
    }
  }
}
