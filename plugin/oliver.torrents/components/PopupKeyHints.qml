import QtQuick
import qs.Commons as Commons
import "../Model.js" as Model
Row {
  spacing:Commons.Style.spacing.lg
  Repeater {
    model:[{key:"a",label:"add"},{key:"/",label:"search"},{key:"o",label:"file"},{key:"O",label:"folder"},{key:"w",label:"window"},{key:"t",label:"terminal"}]
    Row {
      required property var modelData
      spacing:Commons.Style.spacing.sm
      Text {textFormat:Text.PlainText;text:modelData.key;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.caption}
      Text {textFormat:Text.PlainText;text:modelData.label;color:Model.dim(Commons.Color.foreground,Commons.Color.popups.background);font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.caption}
    }
  }
}
