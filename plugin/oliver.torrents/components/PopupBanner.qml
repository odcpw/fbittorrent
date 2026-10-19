import QtQuick
import qs.Commons as Commons
import qs.Ui as Ui
import "../Model.js" as Model
Rectangle {
  id:root
  property string text:""
  property bool urgent:false
  property bool showLog:false
  signal logRequested()
  visible:text!==""
  implicitHeight:visible ? labels.implicitHeight+Commons.Style.spacing.xxl : 0
  color:Commons.Util.alpha(urgent ? Commons.Color.urgent : Commons.Color.foreground,0.10)
  border.color:Commons.Util.alpha(urgent ? Commons.Color.urgent : Commons.Color.foreground,0.35)
  border.width:Commons.Style.space(1)
  radius:Commons.Style.cornerRadius
  opacity:visible ? 1 : 0
  Behavior on opacity {NumberAnimation {duration:Commons.Style.duration(140)}}
  Column {
    id:labels
    anchors {left:parent.left;right:parent.right;verticalCenter:parent.verticalCenter; margins:Commons.Style.spacing.md}
    spacing:Commons.Style.spacing.md
    Text {width:parent.width;textFormat:Text.PlainText;text:root.text;color:root.urgent ? Commons.Color.urgent : Model.dim(Commons.Color.foreground,Commons.Color.popups.background);font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.bodySmall;wrapMode:Text.Wrap}
    Ui.Button {visible:root.showLog;text:"Show log";onClicked:root.logRequested()}
  }
}
