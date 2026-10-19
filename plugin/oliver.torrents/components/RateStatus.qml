import QtQuick
import QtQuick.Layouts
import qs.Commons as Commons
import "../Formats.js" as Formats

RowLayout {
  id:root
  property var service:null
  readonly property var history:service && service.online && Array.isArray(service.status.history) ? service.status.history : []
  readonly property string rateText:service && service.online ? "↓ "+Formats.rate(service.status.dl,false)+"   ↑ "+Formats.rate(service.status.up,false) : "Offline"
  spacing:Commons.Style.spacing.panelGap
  Layout.minimumHeight:24
  height:Math.max(24,implicitHeight)
  Text {text:root.rateText;textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.bodySmall}
  Item {Layout.fillWidth:true}
  Sparkline {objectName:"download-history";Layout.preferredWidth:Math.min(180,root.width/2);Layout.preferredHeight:24;visible:root.history.length>0;samples:root.history;Accessible.description:root.history.length+" one-second samples, oldest to newest"}
}
