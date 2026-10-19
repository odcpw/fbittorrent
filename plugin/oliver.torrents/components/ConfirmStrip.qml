import QtQuick
import qs.Commons as Commons
import qs.Ui as Ui
Column {
  id:root
  property string message:""
  property string confirmText:"Remove"
  property int selected:0
  readonly property bool cancelFocused:cancel.activeFocus
  signal canceled()
  signal confirmed()
  spacing:Commons.Style.spacing.md
  function reset(){selected=0;cancel.forceActiveFocus()}
  function move(dx){selected=Math.max(0,Math.min(1,selected+dx));if(selected===0)cancel.forceActiveFocus();else confirm.forceActiveFocus()}
  function activate(){if(selected===0)canceled();else confirmed()}
  Text {width:parent.width;textFormat:Text.PlainText;text:root.message;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body;wrapMode:Text.Wrap}
  Row {
    spacing:Commons.Style.spacing.md
    Ui.Button {id:cancel;text:"Cancel";focusable:true;bordered:true;onClicked:root.canceled()}
    Ui.Button {id:confirm;text:root.confirmText;focusable:true;bordered:true;onClicked:root.confirmed()}
  }
}
