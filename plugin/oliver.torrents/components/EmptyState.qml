import QtQuick
import QtQuick.Layouts
import qs.Commons as Commons
import "../Model.js" as Model

ColumnLayout {
  property alias title:title.text
  property alias message:message.text
  spacing:Commons.Style.spacing.panelGap
  Text {id:title;Layout.fillWidth:true;textFormat:Text.PlainText;wrapMode:Text.Wrap;horizontalAlignment:Text.AlignHCenter;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.heading;font.bold:true}
  Text {id:message;Layout.fillWidth:true;textFormat:Text.PlainText;wrapMode:Text.Wrap;horizontalAlignment:Text.AlignHCenter;color:Model.dim(Commons.Color.foreground,Commons.Color.background);font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
}
