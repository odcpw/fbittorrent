import QtQuick
import QtQuick.Layouts
import qs.Commons as Commons
import qs.Ui as Ui
import "../components" as Components
import "../Model.js" as Model
FocusScope {
  id:root
  required property var service
  readonly property var flow:service.addFlow
  readonly property var model:{var revision=service.addRevision;return flow.snapshot()}
  property string lastPhase:""
  signal changed()
  function focusSheet(){if(model.phase==="input"){if(model.source)sourceInput.text=model.source.magnet || model.source.path || model.source.url;sourceInput.forceActiveFocus()}else{sourceInput.focus=false;if(!destination.activeFocus && !cancel.activeFocus && !keep.activeFocus)forceActiveFocus()}}
  function handleKey(event){
    if(event.key===Qt.Key_Escape){flow.cancel();return true}
    if(event.key===Qt.Key_Return || event.key===Qt.Key_Enter){
      if(sourceInput.activeFocus)flow.review(sourceInput.text)
      else if(cancel.activeFocus)flow.cancel()
      else if(keep.activeFocus)flow.keep()
      else if(model.phase==="error")flow.retry()
      else flow.commit()
      return true
    }
    return false
  }
  Keys.priority:Keys.BeforeItem
  Keys.onPressed:function(event){if(root.handleKey(event))event.accepted=true}
  onModelChanged:{changed();if(lastPhase!==model.phase){lastPhase=model.phase;Qt.callLater(focusSheet)}}
  Component.onCompleted:Qt.callLater(focusSheet)
  Rectangle {anchors.fill:parent;color:Commons.Util.alpha(Commons.Color.background,0.7);MouseArea {anchors.fill:parent;acceptedButtons:Qt.AllButtons}}
  Ui.BorderSurface {
    anchors.centerIn:parent
    width:Math.min(parent.width-Commons.Style.space(36),Commons.Style.space(560))
    height:Math.min(parent.height-Commons.Style.space(36),body.implicitHeight+Commons.Style.space(36))
    color:Commons.Color.popups.background
    borderSpec:Commons.Border.surfaceSpec("popups","border",Commons.Color.popups.border,Commons.Style.space(2))
    padding:Commons.Style.space(18);radius:Commons.Style.cornerRadius
    ColumnLayout {
      id:body
      anchors.fill:parent;anchors.margins:Commons.Style.space(18)
      spacing:Commons.Style.spacing.xxl
      Text {textFormat:Text.PlainText;text:"Add torrent";color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.title;font.bold:true}
      Ui.TextField {id:sourceInput;visible:root.model.phase==="input" || (root.model.phase==="error" && !root.model.source && !root.model.owned);Layout.fillWidth:true;placeholderText:"Magnet, HTTP URL or .torrent path";maximumLength:65536;onAccepted:root.flow.review(text)}
      Text {visible:root.model.name!=="";Layout.fillWidth:true;textFormat:Text.PlainText;text:(root.model.duplicate ? "Already in your list: " : "")+root.model.name;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body;wrapMode:Text.Wrap;maximumLineCount:3;elide:Text.ElideRight;HoverHandler {id:nameHover} Components.FullTextTip {visible:nameHover.hovered;text:parent.text}}
      Text {
        Layout.fillWidth:true;textFormat:Text.PlainText;wrapMode:Text.Wrap
        text:root.model.phase==="metadata_timeout" ? "No peers answered yet." : root.model.phase==="preview" ? "Metadata will be fetched after Download." : root.model.phase==="staging" || root.model.phase==="resolving" ? "Resolving metadata… "+Math.floor(root.model.elapsed_s/60)+":"+String(root.model.elapsed_s%60).padStart(2,"0") : root.model.phase==="committing" ? "Adding download…" : root.model.phase==="discarding" || root.model.phase==="cancel_wait" ? "Discarding…" : root.model.size!==undefined ? Model.size(root.model.size,false)+" · "+root.model.files.length+" files" : ""
        color:Model.dim(Commons.Color.foreground,Commons.Color.popups.background);font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.bodySmall
      }
      Components.AddFiles {visible:root.model.files.length>0;Layout.fillWidth:true;Layout.preferredHeight:Math.min(implicitHeight,root.model.files.length*Commons.Style.spacing.controlHeight);flow:root.flow;sheet:root.model}
      RowLayout {
        visible:!root.model.duplicate && root.model.phase!=="input";Layout.fillWidth:true;spacing:Commons.Style.spacing.lg
        Text {textFormat:Text.PlainText;text:"Seeding";color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
        Ui.ToggleSwitch {checked:root.model.seed;busy:root.model.busy;onToggled:root.flow.seed(!root.model.seed)}
        Text {Layout.fillWidth:true;textFormat:Text.PlainText;text:root.model.seed ? "Continues seeding when complete" : "Upload capped at 32 KiB/s, stops when complete";wrapMode:Text.Wrap;color:Model.dim(Commons.Color.foreground,Commons.Color.popups.background);font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.bodySmall}
      }
      RowLayout {
        visible:!root.model.duplicate && root.model.phase!=="input";Layout.fillWidth:true
        Text {textFormat:Text.PlainText;text:"Save to";color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
        Ui.TextField {id:destination;Layout.fillWidth:true;text:root.model.savePath;maximumLength:4096;placeholderText:"Default download folder";enabled:!root.model.busy;onTextEdited:root.flow.path(text);onAccepted:root.flow.commit()}
      }
      Text {visible:root.model.error!=="";Layout.fillWidth:true;textFormat:Text.PlainText;text:root.model.error;wrapMode:Text.Wrap;color:Commons.Color.urgent;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.bodySmall}
      RowLayout {
        Layout.fillWidth:true;spacing:Commons.Style.spacing.lg
        Ui.Button {id:cancel;text:root.model.duplicate ? "Close" : "Discard (Esc)";focusable:true;bordered:true;enabled:root.model.phase!=="committing" && root.model.phase!=="discarding";onClicked:root.flow.cancel()}
        Item {Layout.fillWidth:true}
        Ui.Button {id:keep;visible:root.model.phase==="metadata_timeout";text:"Keep paused";focusable:true;bordered:true;onClicked:root.flow.keep()}
        Ui.Button {text:root.model.phase==="error" ? "Review again" : root.model.phase==="input" ? "Review" : root.model.duplicate ? "Open list" : root.model.stopped ? "Add paused (Enter)" : "Download (Enter)";visible:root.model.phase!=="metadata_timeout";enabled:["input","ready","preview","duplicate","error"].indexOf(root.model.phase)>=0;focusable:true;bordered:true;onClicked:if(root.model.phase==="error")root.flow.retry();else if(root.model.phase==="input")root.flow.review(sourceInput.text);else root.flow.commit()}
      }
    }
  }
}
