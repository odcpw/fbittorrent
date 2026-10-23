import QtQuick
import QtQuick.Layouts
import Quickshell
import qs.Commons as Commons
import qs.Ui as Ui
import "../components" as Components
FocusScope {
  id:root
  required property var service
  readonly property var flow:service.overviewFlow
  readonly property var model:service.overviewRevision>=0 ? flow.snapshot() : ({})
  readonly property var row:service.detailRevision>=0 ? service.store.torrents[model.hash] || {} : ({})
  readonly property var state:({modal:model.modal,hash:model.hash,message:model.message,picker:folder.item ? folder.item.state() : null})
  signal changed()
  onStateChanged:changed()
  function handleKey(event){
    if(event.key===Qt.Key_Escape){flow.cancel();return true}
    if(model.modal==="move"){if(folder.item)return folder.item.handleKey(event);return true}
    if(event.key===Qt.Key_Return || event.key===Qt.Key_Enter){flow.limit(limit.text);return true}
    return false
  }
  Keys.priority:Keys.BeforeItem
  Keys.onPressed:function(event){if(root.handleKey(event))event.accepted=true}
  Component.onCompleted:Qt.callLater(function(){if(model.modal==="limit"){limit.text=row.download_limit_bps===undefined ? "" : String(row.download_limit_bps);limit.forceActiveFocus()}})
  Rectangle {anchors.fill:parent;color:Commons.Util.alpha(Commons.Color.background,0.8);MouseArea {anchors.fill:parent;acceptedButtons:Qt.AllButtons}}
  Components.PaneFrame {
    anchors.centerIn:parent;width:Math.min(parent.width-Commons.Style.space(36),Commons.Style.space(600));height:Math.min(parent.height-Commons.Style.space(36),Commons.Style.space(root.model.modal==="move" ? 440 : 290))
    Loader {
      id:folder;anchors.fill:parent;active:root.model.modal==="move"
      onLoaded:{item.forceActiveFocus();root.changed()}
      sourceComponent:Components.FolderPicker {
        initialPath:root.row.save_path || Quickshell.env("HOME")
        onChanged:root.changed()
        onSelected:function(path){root.flow.move(path)}
        onCancelled:root.flow.cancel()
      }
    }
    ColumnLayout {
      anchors.fill:parent;visible:root.model.modal==="limit";spacing:Commons.Style.spacing.md
      Text {text:"Download cap";textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.heading;font.bold:true}
      Text {Layout.fillWidth:true;text:root.row.name || "";textFormat:Text.PlainText;elide:Text.ElideRight;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
      Ui.TextField {id:limit;Layout.fillWidth:true;maximumLength:16;placeholderText:"Bytes/s, K or M; empty or none = unlimited"}
      Text {Layout.fillWidth:true;text:root.model.message || "Minimum finite cap: 16K/s. The active global limit may be lower.";textFormat:Text.PlainText;wrapMode:Text.Wrap;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
      RowLayout {Ui.Button {text:"Cancel (Esc)";onClicked:root.flow.cancel()} Item {Layout.fillWidth:true} Ui.Button {text:"Apply (Enter)";onClicked:root.flow.limit(limit.text)}}
    }
  }
}
