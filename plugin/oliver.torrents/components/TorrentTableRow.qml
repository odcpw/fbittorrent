import QtQuick
import qs.Ui as Ui
import qs.Commons as Commons

Ui.BorderSurface {
  id:root
  required property var host
  required property var torrent
  property bool selected:false
  readonly property bool hasCursor:host.cursorHash===torrent.hash
  function progress(){for(var i=0;i<cells.count;i++){var cell=cells.itemAt(i);if(cell && cell.column.name==="progress")return cell.renderedProgress}return 0}
  function stateText(){for(var i=0;i<cells.count;i++){var cell=cells.itemAt(i);if(cell && cell.column.name==="state")return cell.renderedState}return ""}
  height:Commons.Style.spacing.popupRowHeight
  color:hasCursor ? Commons.Style.selectedAccentFill : selected ? Commons.Style.selectionFill : "transparent"
  borderSpec:Commons.Border.none()
  radius:Commons.Style.cornerRadius
  Rectangle {visible:root.hasCursor;anchors.left:parent.left;anchors.top:parent.top;anchors.bottom:parent.bottom;width:Commons.Style.space(3);color:Commons.Color.accent}
  Row {
    anchors.fill:parent
    Repeater {
      id:cells
      model:root.host.columns
      TorrentCell {required property var modelData;width:modelData.width;height:root.height;visible:!modelData.hidden;column:modelData;torrent:root.torrent;pendingText:root.host.service.pendingGlyph(root.torrent.hash);failed:root.host.service.rowFailed(root.torrent.hash)}
    }
  }
  MouseArea {
    anchors.fill:parent
    acceptedButtons:Qt.LeftButton
    hoverEnabled:true
    onEntered:{root.host.hoverHash=root.torrent.hash;root.host.hovered(root.torrent.hash)}
    onExited:if(root.host.hoverHash===root.torrent.hash)root.host.hoverHash=""
    onClicked:function(mouse){root.host.chosen(root.torrent.hash,mouse.modifiers)}
    onDoubleClicked:{root.host.activated(root.torrent.hash)}
  }
}
