import QtQuick
import QtQuick.Layouts
import qs.Ui as Ui
import qs.Commons as Commons
import "../TrackerText.js" as TrackerText
FocusScope {
  id:root
  required property var flow
  property string mode:""
  property string target:""
  property string error:""
  readonly property string confirmationText:"Remove tracker "+TrackerText.publicUrl(root.target)+"?"
  readonly property bool opened:mode!==""
  readonly property bool cancelFocused:mode==="remove" ? confirm.selectedIndex===0 : cancel.activeFocus
  visible:opened
  function show(add,url){
    target=url || "";error="";input.text="";mode=add ? "add" : "remove";confirm.selectedIndex=0
    Qt.callLater(function(){
      input.focus=false;cancel.focus=false;addButton.focus=false
      if(add)input.forceActiveFocus();else confirm.forceActiveFocus()
    })
  }
  function close(){mode="";target="";input.text="";root.focus=false}
  function submit(){
    var add=mode==="add",value=add ? input.text.trim() : target
    if(!value){error="Enter a tracker URL";return}
    // Validation errors carry no input; raw URLs never become a receipt/error.
    if(!root.flow.editTracker(value,add,function(result,failure){if(failure)error="Tracker edit failed · check message history"})){error="Tracker edit is unavailable";return}
    close()
  }
  function key(event){
    if(!opened)return false
    if(mode==="remove")return confirm.handleKey(event) || true
    if(event.key===Qt.Key_Escape){close();return true}
    if(event.key===Qt.Key_Return || event.key===Qt.Key_Enter){if(cancel.activeFocus)close();else submit();return true}
    if(event.key===Qt.Key_Tab || event.key===Qt.Key_Backtab){if(input.activeFocus)cancel.forceActiveFocus();else if(cancel.activeFocus)addButton.forceActiveFocus();else input.forceActiveFocus();return true}
    // The focused text field owns printable input. Stop ancestor shortcuts.
    return false
  }
  Keys.priority:Keys.BeforeItem
  Keys.onPressed:function(event){if(root.key(event))event.accepted=true}
  Rectangle {anchors.fill:parent;color:Commons.Util.alpha(Commons.Color.background,.9);MouseArea {anchors.fill:parent}}
  Ui.BorderSurface {
    visible:root.mode==="add";anchors.centerIn:parent;width:Math.max(0,parent.width-Commons.Style.space(16));height:body.implicitHeight+Commons.Style.space(32)
    color:Commons.Color.background;borderSpec:Commons.Border.flat(Commons.Color.accent,Commons.Style.normalBorderWidth);padding:Commons.Style.space(16)
    ColumnLayout {
      id:body;anchors.fill:parent;anchors.margins:Commons.Style.space(16);spacing:Commons.Style.spacing.lg
      Text {Layout.fillWidth:true;text:"Add tracker · input hidden";textFormat:Text.PlainText;wrapMode:Text.Wrap;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
      Ui.TextField {id:input;objectName:"tracker-url-input";Layout.fillWidth:true;password:true;maximumLength:4096;placeholderText:"http(s) or udp URL"}
      Text {Layout.fillWidth:true;visible:!!root.error;text:root.error;textFormat:Text.PlainText;wrapMode:Text.Wrap;color:Commons.Color.urgent;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
      RowLayout {
        Layout.fillWidth:true
        Ui.Button {id:cancel;objectName:"tracker-add-cancel";text:"Cancel";focusable:true;onClicked:root.close()}
        Item {Layout.fillWidth:true}
        Ui.Button {id:addButton;objectName:"tracker-add-submit";text:"Add tracker";focusable:true;onClicked:if(root.mode==="add")root.submit()}
      }
    }
  }
  Ui.ConfirmDialog {
    id:confirm;anchors.fill:parent;opened:root.mode==="remove";selectedIndex:0
    confirmText:"Remove tracker";message:root.confirmationText
    onCanceled:root.close();onConfirmed:root.submit()
  }
}
