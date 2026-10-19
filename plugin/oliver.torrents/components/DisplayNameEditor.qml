import QtQuick
import QtQuick.Layouts
import qs.Ui as Ui
import qs.Commons as Commons

ColumnLayout {
  id:root
  required property var service
  required property var torrent
  property bool editing:false
  property bool pending:false
  property string error:""
  readonly property string hash:torrent.hash
  onHashChanged:{editing=false;pending=false;error=""}
  function begin(){if(!service.hasCap("rename"))return;error="";input.text=torrent.name;editing=true;input.forceActiveFocus();input.selectAll()}
  function cancel(){if(!pending){editing=false;error=""}}
  function key(event){if(!editing)return false;if(event.key===Qt.Key_Escape){cancel();return true}return false}
  function save(){
    if(pending)return
    var name=input.text
    if(/[\x00-\x1f\x7f-\x9f\u2028\u2029]/.test(name)){error="Names cannot contain control characters";return}
    var selected=hash
    pending=true;error=""
    if(!service.mutate("torrents.rename",{hash:torrent.hash,name:name},"rename",function(result,failure){if(hash!==selected)return;pending=false;if(failure)error=failure.message;else editing=false})){pending=false;error=service.lastError || "Could not send rename"}
  }
  Text {
    Layout.fillWidth:true;visible:!root.editing;text:root.torrent.name;textFormat:Text.PlainText
    font.bold:true;wrapMode:Text.Wrap;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body
    MouseArea {anchors.fill:parent;onDoubleClicked:root.begin()}
  }
  Ui.TextField {id:input;objectName:"displayNameInput";Layout.fillWidth:true;visible:root.editing;enabled:!root.pending;maximumLength:512;onAccepted:root.save();Keys.onEscapePressed:root.cancel()}
  RowLayout {visible:root.editing
    Ui.Button {text:root.pending ? "Saving…" : "Save";enabled:!root.pending;onClicked:root.save()}
    Ui.Button {text:"Cancel";enabled:!root.pending;onClicked:root.cancel()}
  }
  Text {Layout.fillWidth:true;visible:root.editing || !!root.error;text:root.error || "Empty restores the original name. Payload files keep their names.";wrapMode:Text.Wrap;textFormat:Text.PlainText;color:root.error ? Commons.Color.urgent : Commons.Color.foreground;font.pixelSize:Commons.Style.font.caption}
}
