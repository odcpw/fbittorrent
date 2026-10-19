import QtQuick
import QtQuick.Layouts
import qs.Commons as Commons
import qs.Ui as Ui
import "../SettingsSchema.js" as Schema
ColumnLayout {
 id:root
 required property var spec
 required property var service
 readonly property var flow:service.settingsFlow
 readonly property var value:service.settingsRevision>=0 ? service.store.settings[spec.key] : undefined
 readonly property var error:service.settingsRevision>=0 ? flow.errors[spec.key] || "" : ""
 readonly property bool pending:service.settingsRevision>=0 && !!flow.pending[spec.key]
 readonly property bool editing:!!textField && textField.activeFocus
 readonly property var textField:editor.item
 readonly property var toggle:switchControl.item
 readonly property var dropdown:choices.item
 readonly property var options:{var revision=service.settingsRevision;return spec.kind==="interface" ? [{value:"",label:"Off — no binding"}].concat(flow.interfaces.map(function(name){return {value:name,label:name}})).concat(value && flow.interfaces.indexOf(value)<0 ? [{value:value,label:value+" (not present)"}] : []) : spec.options || []}
 onPendingChanged:if(!pending && !flow.errors[spec.key] && textField)textField.text=Schema.text(service.store.settings[spec.key])
 onValueChanged:if(textField && !textField.activeFocus)textField.text=Schema.text(value)
 spacing:Commons.Style.spacing.sm
 RowLayout {
  Layout.fillWidth:true;spacing:Commons.Style.spacing.lg
  Text {Layout.fillWidth:true;Layout.preferredWidth:Commons.Style.space(260);textFormat:Text.PlainText;text:root.spec.label;wrapMode:Text.Wrap;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
  Loader {id:switchControl;active:root.spec.kind==="bool";visible:active;sourceComponent:Ui.ToggleSwitch {checked:root.value===true;busy:root.pending;activeFocusOnTab:true;hasCursor:activeFocus;Keys.onSpacePressed:root.flow.set(root.spec.key,!root.value);onToggled:root.flow.set(root.spec.key,!root.value)}}
  Loader {id:choices;active:root.spec.kind==="choice" || root.spec.kind==="interface";visible:active;Layout.preferredWidth:Commons.Style.space(240);sourceComponent:Ui.Dropdown {showLabel:false;options:root.options;value:Schema.text(root.value);enabled:!root.pending;onChanged:function(value){root.flow.set(root.spec.key,value)}}}
  Loader {id:editor;active:["text","number","limit"].indexOf(root.spec.kind)>=0;visible:active;Layout.fillWidth:true;Layout.preferredWidth:Commons.Style.space(240);sourceComponent:Ui.TextField {text:Schema.text(root.value);maximumLength:4096;placeholderText:root.spec.kind==="limit" ? "Unlimited (empty)" : "";enabled:!root.pending;onAccepted:root.flow.set(root.spec.key,text)}}
  Ui.Button {visible:!!root.textField;text:root.pending ? "Applying…" : "Apply";focusable:true;enabled:!!root.textField && !root.pending && root.textField.text!==Schema.text(root.value);onClicked:root.flow.set(root.spec.key,root.textField.text)}
 }
 Text {Layout.fillWidth:true;visible:root.error!=="";text:root.error;textFormat:Text.PlainText;wrapMode:Text.Wrap;color:Commons.Color.urgent;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.bodySmall}
}
