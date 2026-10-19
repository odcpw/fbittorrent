import QtQuick
import QtQuick.Layouts
import qs.Commons as Commons
import "../DetailText.js" as DetailText

ColumnLayout {
  id:root
  property var card:null
  readonly property bool loaded:poster.status===Image.Ready
  spacing:Commons.Style.spacing.sm
  visible:!!card
  RowLayout {
    Layout.fillWidth:true
    spacing:Commons.Style.spacing.sm
    Image {
      id:poster
      visible:!!(root.card && root.card.poster_path)
      Layout.preferredWidth:Commons.Style.space(80);Layout.preferredHeight:Commons.Style.space(120)
      source:DetailText.localUrl(root.card ? root.card.poster_path : "")
      asynchronous:true;sourceSize:Qt.size(Commons.Style.space(160),Commons.Style.space(240));fillMode:Image.PreserveAspectFit
    }
    Text {
      Layout.fillWidth:true;wrapMode:Text.Wrap;textFormat:Text.PlainText
      text:root.card ? root.card.title+(root.card.year ? " ("+root.card.year+")" : "")+(root.card.rating!==undefined ? "\nIMDb "+root.card.rating : "") : ""
      color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.title;font.bold:true
    }
  }
  Text {Layout.fillWidth:true;wrapMode:Text.Wrap;textFormat:Text.PlainText;text:root.card ? root.card.blurb || "" : "";color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
}
