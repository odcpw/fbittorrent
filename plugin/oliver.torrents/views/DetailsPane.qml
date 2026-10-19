import QtQuick
import QtQuick.Layouts
import qs.Ui as Ui
import qs.Commons as Commons
import "../components" as Components

ColumnLayout {
  id:root
  required property var service
  property bool focused:false
  readonly property var flow:service.detailFlow
  readonly property var torrent:service.detailRevision>=0 && flow ? flow.row() : null
  readonly property string tab:service.detailRevision>=0 && flow ? flow.tab : "overview"
  readonly property var overviewState:overview.item ? overview.item.actionState : null
  readonly property bool posterLoaded:overview.item ? overview.item.posterLoaded : false
  readonly property bool editing:files.item ? files.item.editing : false
  signal recheckRequested(string hash)
  function key(event){if(files.item && files.item.key(event))return true;return false}
  spacing:Commons.Style.spacing.sm
  Rectangle {Layout.fillWidth:true;Layout.preferredHeight:Commons.Style.space(2);color:Commons.Color.accent;opacity:root.focused ? 1 : 0}
  Flow {
    Layout.fillWidth:true;height:childrenRect.height
    spacing:Commons.Style.spacing.xs
    Repeater {model:root.service.detailRevision>=0 && root.flow ? root.flow.tabs() : [];Ui.Button {required property string modelData;text:modelData.charAt(0).toUpperCase()+modelData.slice(1);hasCursor:root.tab===modelData;onClicked:root.flow.selectTab(modelData)}}
  }
  Text {visible:!root.torrent;Layout.fillWidth:true;text:"Select a torrent and press Enter.";wrapMode:Text.Wrap;textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
  Loader {id:overview;Layout.fillWidth:true;Layout.fillHeight:true;active:!!root.torrent && root.tab==="overview";visible:active;sourceComponent:DetailsOverview {service:root.service;torrent:root.torrent;card:root.service.detailRevision>=0 ? root.flow.card : null;onRecheckRequested:function(hash){root.recheckRequested(hash)}}}
  Loader {id:files;Layout.fillWidth:true;Layout.fillHeight:true;active:!!root.torrent && root.tab==="files";visible:active;sourceComponent:DetailsFiles {service:root.service;flow:root.flow;torrent:root.torrent}}
  Loader {Layout.fillWidth:true;Layout.fillHeight:true;active:!!root.torrent && root.tab==="log";visible:active;sourceComponent:LogView {service:root.service;hash:root.flow.hash;consumer:"detail-log"}}
  Loader {Layout.fillWidth:true;Layout.fillHeight:true;active:!!root.torrent && ["overview","files","log"].indexOf(root.tab)<0;visible:active;sourceComponent:DetailCollection {service:root.service;flow:root.flow}}

  Components.KeyHint {Layout.fillWidth:true;text:"[ / ] tabs · h table · Esc back"}
}
