import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import qs.Commons as Commons
import qs.Ui as Ui
import "../components" as Components
import "../SettingsSchema.js" as Schema
FocusScope {
 id:root
 required property var service
 readonly property var flow:service.settingsFlow
 readonly property var state:service.settingsRevision>=0 ? flow.snapshot() : ({})
 readonly property bool editing:rowsHaveFocus()
 signal dismissed()
 function rowsHaveFocus(){for(var i=0;i<rows.count;i++)if(rows.itemAt(i).editing)return true;return false}
 function row(key){var index=Schema.fields.findIndex(function(field){return field.key===key});return index>=0 ? rows.itemAt(index) : null}
 function focusSetting(key){var item=row(key);if(!item)return false;scroll.contentY=Math.max(0,item.y-Commons.Style.spacing.lg);if(item.textField.visible)item.textField.forceActiveFocus();else if(item.toggle.visible)item.toggle.forceActiveFocus();return true}
 function focusSheet(){forceActiveFocus()}
 Component.onCompleted:flow.open()
 Component.onDestruction:flow.close()
 Keys.priority:Keys.BeforeItem
 Keys.onPressed:function(event){if(event.key===Qt.Key_Escape){dismissed();event.accepted=true}}
 Rectangle {anchors.fill:parent;color:Commons.Util.alpha(Commons.Color.background,0.7);MouseArea {anchors.fill:parent;acceptedButtons:Qt.AllButtons}}
 Ui.BorderSurface {
  anchors.centerIn:parent;width:Math.min(parent.width-Commons.Style.space(36),Commons.Style.space(780));height:Math.min(parent.height-Commons.Style.space(36),Commons.Style.space(760))
  color:Commons.Color.popups.background;borderSpec:Commons.Border.surfaceSpec("popups","border",Commons.Color.popups.border,Commons.Style.space(2));radius:Commons.Style.cornerRadius
  ColumnLayout {
   anchors.fill:parent;anchors.margins:Commons.Style.space(18);spacing:Commons.Style.spacing.lg
   RowLayout {Layout.fillWidth:true;Text {Layout.fillWidth:true;text:"Settings";textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.title;font.bold:true} Ui.Button {text:"Close (Esc)";focusable:true;onClicked:root.dismissed()}}
   Components.PopupBanner {Layout.fillWidth:true;text:root.state.banner || ""}
   Text {Layout.fillWidth:true;visible:!!(root.state.errors.load || root.state.errors.network);text:root.state.errors.load || root.state.errors.network || "";textFormat:Text.PlainText;color:Commons.Color.urgent;wrapMode:Text.Wrap;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.bodySmall}
   Text {Layout.fillWidth:true;text:"Changes apply immediately. Limit fields accept bytes/s, K or M; empty means unlimited.";textFormat:Text.PlainText;wrapMode:Text.Wrap;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.bodySmall}
   Flickable {
    id:scroll;Layout.fillWidth:true;Layout.fillHeight:true;clip:true;contentWidth:width;contentHeight:fields.implicitHeight;boundsBehavior:Flickable.StopAtBounds
    ScrollBar.vertical:ScrollBar {}
    ColumnLayout {id:fields;width:scroll.width;spacing:Commons.Style.spacing.xxl
     Repeater {id:rows;model:Schema.fields;Components.SettingRow {required property var modelData;spec:modelData;service:root.service;Layout.fillWidth:true}}
    }
   }
  }
 }
}
