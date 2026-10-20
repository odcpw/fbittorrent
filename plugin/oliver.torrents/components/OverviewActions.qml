import QtQuick
import QtQuick.Layouts
import qs.Commons as Commons
import qs.Ui as Ui
import "../Model.js" as Model
import "../OverviewFlow.js" as Overview
ColumnLayout {
  id:root
  required property var service
  required property var torrent
  readonly property var flow:service.overviewFlow
  readonly property string status:service.overviewRevision>=0 ? flow.status(torrent.hash) : ""
  readonly property bool busy:service.overviewRevision>=0 && (flow.busy(torrent.hash) || service.rowPending(torrent.hash))
  readonly property var effective:{var confirmed=service.settingsRevision;return Overview.effectiveCap(torrent,service.store.settings,service.turtleOn)}
  readonly property var state:({cap:capField.value,effective:effectiveField.value,status:status})
  Layout.fillWidth:true
  RowLayout {
    Ui.Button {objectName:"torrent-retry";visible:root.torrent.state==="error";text:"Retry (p)";enabled:!root.busy;onClicked:root.service.resume(root.torrent.hash)}
    Ui.Button {visible:root.service.hasCap("move");text:"Move… (m)";enabled:!root.busy;onClicked:root.flow.open(root.torrent.hash,"move")}
    Ui.Button {text:"Refresh status";onClicked:root.flow.refresh(root.torrent.hash)}
  }
  DetailField {id:capField;Layout.fillWidth:true;label:"Download cap";value:root.torrent.download_limit_bps===undefined ? "Unlimited" : Model.rate(root.torrent.download_limit_bps,false)}
  DetailField {id:effectiveField;Layout.fillWidth:true;label:"Effective cap";value:root.effective===undefined ? "—" : root.effective===null ? "Unlimited" : Model.rate(root.effective,false)}
  Ui.Button {visible:root.service.hasCap("download_limit");text:"Set download cap… (c)";enabled:!root.busy;onClicked:root.flow.open(root.torrent.hash,"limit")}
  Text {visible:!!root.status;Layout.fillWidth:true;text:root.status;textFormat:Text.PlainText;wrapMode:Text.Wrap;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
}
