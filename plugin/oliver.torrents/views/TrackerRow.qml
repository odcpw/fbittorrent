import QtQuick
import QtQuick.Layouts
import qs.Commons as Commons
import "../Model.js" as Model
import "../TrackerText.js" as TrackerText
ColumnLayout {
  id:root
  required property var tracker
  property real now:Date.now()
  spacing:Commons.Style.spacing.xs
  function rendered(){return [url.text,status.text,announces.text,message.text].join("\n")}
  Text {
    id:url
    Layout.fillWidth:true;text:TrackerText.publicUrl(root.tracker.url);textFormat:Text.PlainText;wrapMode:Text.WrapAnywhere
    color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body
  }
  Text {
    id:status
    Layout.fillWidth:true;text:TrackerText.status(root.tracker.status)+" · Peers "+Model.count(root.tracker.peers);textFormat:Text.PlainText;wrapMode:Text.Wrap
    color:root.tracker.status==="error" ? Commons.Color.urgent : Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body
  }
  Text {
    id:announces
    Layout.fillWidth:true;text:{var clock=root.now;return "Last "+Model.when(root.tracker.last_announce_at)+" · Next "+Model.when(root.tracker.next_announce_at)};textFormat:Text.PlainText;wrapMode:Text.Wrap
    color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.bodySmall
  }
  Text {
    id:message
    Layout.fillWidth:true;visible:!!text;text:TrackerText.message(root.tracker.message,root.tracker.url);textFormat:Text.PlainText;wrapMode:Text.Wrap
    color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body
  }
}
