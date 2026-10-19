import QtQuick
import qs.Ui as Ui
import qs.Commons as Commons

Row {
  id:root
  property var badges:({})
  property var views:["torrents","search","shows","rss","settings","log"]
  property var labels:["Torrents","Search","Shows","RSS","Settings","Log"]
  signal chosen(int index)
  spacing:Commons.Style.spacing.xxs
  Repeater {
    model:root.views
    Row {
      required property int index
      required property string modelData
      spacing:Commons.Style.spacing.xxs
      Ui.Button {text:(index<4 ? (index+1)+" " : "")+root.labels[index];onClicked:root.chosen(index)}
      Text {
        anchors.verticalCenter:parent.verticalCenter
        visible:Number(root.badges[modelData])>0
        text:"●"+root.badges[modelData]
        textFormat:Text.PlainText
        color:Commons.Color.accent;font.bold:true
        font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.bodySmall
      }
    }
  }
}
