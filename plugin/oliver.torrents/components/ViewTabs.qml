import QtQuick
import qs.Ui as Ui
import qs.Commons as Commons

Row {
  id:root
  property var badges:({})
  required property string currentView
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
      Ui.Button {
        text:(index<4 ? (index+1)+" " : "")+root.labels[index]
        selected:root.currentView===modelData
        onClicked:root.chosen(index)
        Rectangle {
          anchors.left:parent.left;anchors.right:parent.right;anchors.bottom:parent.bottom
          height:Commons.Style.space(2);visible:parent.selected;color:Commons.Color.accent
        }
      }
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
