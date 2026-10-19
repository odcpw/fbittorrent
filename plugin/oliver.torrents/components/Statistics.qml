import QtQuick
import QtQuick.Layouts
import qs.Commons as Commons
import "../Formats.js" as Formats

ColumnLayout {
  id:root
  property var statistics:null
  property bool online:true
  readonly property bool known:online && !!statistics && !!statistics.session && !!statistics.all_time
  readonly property string sessionText:known ? "Session   ↓ "+Formats.size(statistics.session.downloaded,false)+"   ↑ "+Formats.size(statistics.session.uploaded,false) : online ? "Loading statistics…" : "Statistics unavailable while offline"
  readonly property string allTimeText:known ? "All time   ↓ "+Formats.size(statistics.all_time.downloaded,false)+"   ↑ "+Formats.size(statistics.all_time.uploaded,false)+"   ·   "+Formats.count(statistics.all_time.completed)+" completed" : ""
  spacing:Commons.Style.spacing.sm
  Text {text:"Statistics";textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body;font.bold:true}
  Text {Layout.fillWidth:true;text:root.sessionText;textFormat:Text.PlainText;wrapMode:Text.Wrap;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.bodySmall}
  Text {Layout.fillWidth:true;visible:root.known;text:root.allTimeText;textFormat:Text.PlainText;wrapMode:Text.Wrap;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.bodySmall}
}
