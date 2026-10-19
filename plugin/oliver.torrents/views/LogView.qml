import QtQuick
import QtQuick.Controls
import qs.Commons as Commons
import "../Model.js" as Model

ListView {
  id:root
  required property var service
  property string hash:""
  property string consumer:"log-view"
  property bool consuming:visible
  readonly property var entries:service.logRevision>=0 ? service.store.logEntries(hash) : []
  clip:true;spacing:Commons.Style.spacing.sm
  model:entries
  onConsumingChanged:service.logConsumer(consumer,consuming)
  Component.onCompleted:service.logConsumer(consumer,consuming)
  Component.onDestruction:service.logConsumer(consumer,false)
  delegate:Text {
    required property var modelData
    width:ListView.view.width
    text:Model.when(modelData.ts)+" · "+modelData.level+" · "+modelData.source+"\n"+modelData.message
    textFormat:Text.PlainText;wrapMode:Text.Wrap;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body
  }
  Text {anchors.centerIn:parent;width:parent.width;wrapMode:Text.Wrap;text:!root.service.hasCap("log") ? "Log is unavailable; update the daemon" : "No log entries for this torrent";visible:root.count===0;textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
}
