import QtQuick
import QtQuick.Layouts
import qs.Commons as Commons
import "../Model.js" as Model
import "../Glyphs.js" as Glyphs

RowLayout {
  id:root
  property var value:undefined
  property bool complete:false
  readonly property string caption:complete || (known && value>=1) ? "done" : Model.percent(value,false).replace(/\.\d+%$/,"%")
  readonly property bool known:value!==undefined && value!==null
  spacing:Commons.Style.spacing.sm
  Item {
    Layout.fillWidth:true
    implicitHeight:Commons.Style.font.subtitle
    clip:true
    Text {id:bar;text:"████████";textFormat:Text.PlainText;color:Commons.Style.normalFill;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.subtitle}
    Item {
      width:root.known ? Math.min(parent.width,bar.implicitWidth)*Math.max(0,Math.min(1,root.value)) : 0
      height:parent.height;clip:true
      Text {text:Glyphs.blocks(root.value);textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.subtitle}
      Behavior on width {NumberAnimation {duration:Commons.Style.duration(400);easing.type:Easing.OutCubic}}
    }
  }
  Text {
    text:root.caption
    textFormat:Text.PlainText
    color:root.known ? Commons.Color.foreground : Model.dim(Commons.Color.foreground,Commons.Color.background)
    font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.bodySmall
    Layout.alignment:Qt.AlignRight
  }
}
