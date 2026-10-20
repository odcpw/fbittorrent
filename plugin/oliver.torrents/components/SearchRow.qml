import QtQuick
import QtQuick.Layouts
import qs.Commons as Commons
import "../Model.js" as Model

Rectangle {
  id:root
  required property var row
  property bool selected:false
  property bool marked:false
  property var ratingValue:row.rating
  signal chosen()
  signal toggled()
  signal ranged()
  signal activated()
  implicitHeight:Commons.Style.space(50)
  color:selected ? Commons.Style.selectedFill : marked ? Commons.Util.alpha(Commons.Color.accent,0.12) : Commons.Color.background
  RowLayout {
    anchors.fill:parent;anchors.margins:Commons.Style.spacing.xs;spacing:Commons.Style.spacing.sm
    Text {Layout.fillWidth:true;text:(root.marked ? "✓ " : "")+(root.row.added ? "+ In your list · " : root.row.adding ? "Adding… · " : "")+root.row.name;textFormat:Text.PlainText;elide:Text.ElideRight;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body;HoverHandler {id:nameHover} FullTextTip {visible:nameHover.hovered;text:parent.text}}
    Text {Layout.preferredWidth:Commons.Style.space(76);text:Model.size(root.row.size,true);textFormat:Text.PlainText;horizontalAlignment:Text.AlignRight;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
    Text {Layout.preferredWidth:Commons.Style.space(62);text:Model.count(root.row.seeds);textFormat:Text.PlainText;horizontalAlignment:Text.AlignRight;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
    Text {Layout.preferredWidth:Commons.Style.space(52);text:root.ratingValue===undefined ? "—" : Number(root.ratingValue).toFixed(1);textFormat:Text.PlainText;horizontalAlignment:Text.AlignRight;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
    Text {Layout.preferredWidth:Commons.Style.space(110);text:Model.when(root.row.uploaded_at);textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
    Text {Layout.preferredWidth:Commons.Style.space(130);text:root.row.providers.join(", ");textFormat:Text.PlainText;elide:Text.ElideRight;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body;HoverHandler {id:providerHover} FullTextTip {visible:providerHover.hovered;text:parent.text}}
  }
  MouseArea {anchors.fill:parent;onClicked:function(mouse){if(mouse.modifiers & Qt.ControlModifier)root.toggled();else if(mouse.modifiers & Qt.ShiftModifier)root.ranged();else root.chosen()} onDoubleClicked:root.activated()}
}
