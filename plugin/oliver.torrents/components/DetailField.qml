import QtQuick
import qs.Commons as Commons

Text {
  property string label:""
  property string value:""
  text:label ? label+": "+value : value
  textFormat:Text.PlainText;wrapMode:Text.Wrap
  color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body
}
