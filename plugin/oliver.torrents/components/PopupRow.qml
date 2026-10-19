import QtQuick
import QtQuick.Layouts
import qs.Commons
import qs.Commons as Commons
import qs.Ui
import "../Model.js" as Model
import "../Glyphs.js" as Glyphs
import "../PopupState.js" as PopupState
CursorSurface {
  id:root
  required property var host
  required property var torrent
  required property int rowIdx
  readonly property var svc:host.svc
  readonly property bool pending:{var revision=svc.actionRevision;return svc.rowPending(torrent.hash)}
  readonly property var action:{var revision=svc.actionRevision;return svc.pendingAction(torrent.hash)}
  readonly property bool parked:host.presentation.dim
  readonly property var accentSpec:Model.accentText(Commons.Color.accent,Commons.Color.popups.background,host.foreground)
  readonly property bool actionsVisible:hover.hovered || hasCursor
  hasCursor:host.cursorActive && host.focusSection==="rows" && host.rowIndex===rowIdx
  foreground:host.foreground
  implicitHeight:content.implicitHeight+Style.spacing.rowPaddingX
  opacity:parked ? 0.55 : 1
  HoverHandler {id:hover;onHoveredChanged:if(hovered)host.setRowCursor(root.rowIdx)}
  MouseArea {anchors.fill:parent;acceptedButtons:Qt.LeftButton|Qt.MiddleButton;onClicked:function(mouse){if(mouse.button===Qt.MiddleButton)svc.toggleTorrent(root.torrent);else svc.openTorrent(root.torrent)}}
  Column {
    id:content
    anchors {left:parent.left;right:parent.right;verticalCenter:parent.verticalCenter;leftMargin:Style.spacing.md;rightMargin:Style.spacing.md}
    spacing:Style.spacing.xs
    RowLayout {
      width:parent.width;spacing:Style.spacing.lg
      Text {Layout.preferredWidth:Style.space(26);textFormat:Text.PlainText;text:Model.glyph(root.torrent);color:root.torrent.state==="error" ? Commons.Color.urgent : host.foreground;font.family:Style.font.family;font.pixelSize:Style.font.icon}
      Text {
        id:name;Layout.fillWidth:true;textFormat:Text.PlainText;text:root.torrent.name;elide:Text.ElideMiddle
        color:root.torrent.state==="done" ? host.dim : host.foreground;opacity:root.pending ? 0.55 : 1
        font.family:Style.font.family;font.pixelSize:Style.font.body
        FullTextTip {visible:hover.hovered;text:name.text+"\n"+meta.text}
      }
      Row {
        opacity:root.actionsVisible ? 1 : 0;enabled:root.actionsVisible && !root.pending;spacing:Style.spacing.xxs
        PanelActionButton {iconText:Model.isStopped(root.torrent) ? Glyphs.play : Glyphs.paused;tooltipText:"Pause/resume (p)";onClicked:svc.toggleTorrent(root.torrent)}
        PanelActionButton {iconText:Glyphs.folder;tooltipText:"Open (o)";onClicked:svc.openTorrent(root.torrent)}
        PanelActionButton {iconText:Glyphs.trash;tooltipText:"Remove, keep files (d)";onClicked:host.requestRemoval(root.torrent)}
      }
    }
    Item {
      width:parent.width;implicitHeight:Style.space(3)
      visible:!root.parked && root.torrent.progress!==undefined && (root.torrent.progress<1 || Model.isActive(root.torrent))
      Rectangle {anchors.fill:parent;radius:height/2;color:Commons.Util.alpha(host.foreground,0.12)}
      Rectangle {width:parent.width*Math.max(0,Math.min(1,root.torrent.progress || 0));height:parent.height;radius:height/2;color:Model.isActive(root.torrent) ? host.foreground : host.dim;Behavior on width {NumberAnimation {duration:Style.duration(400);easing.type:Easing.OutCubic}}}
    }
    Text {
      id:meta;objectName:"popup-row-meta";width:parent.width;textFormat:Text.PlainText
      text:root.pending && root.action ? svc.pendingGlyph(root.torrent.hash)+" "+PopupState.verb(root.action) : PopupState.meta(root.torrent,root.parked)
      color:root.torrent.playable && root.torrent.playable.ready ? root.accentSpec.color : host.dim
      font.family:Style.font.family;font.pixelSize:Style.font.caption;font.italic:root.pending;font.bold:!!root.torrent.playable && root.torrent.playable.ready && root.accentSpec.bold;wrapMode:Text.Wrap;maximumLineCount:3;elide:Text.ElideRight
    }
    ConfirmStrip {
      id:removeStrip
      width:parent.width;visible:!!host.pendingRemoval && host.pendingRemoval.hash===root.torrent.hash
      message:visible ? "Remove '"+root.torrent.name+"'? Files are kept." : ""
      onVisibleChanged:if(visible)Qt.callLater(reset)
      onCanceled:host.cancelRemove()
      onConfirmed:host.confirmRemove()
      Component.onCompleted:host.registerConfirm(root.torrent.hash,removeStrip)
      Component.onDestruction:host.unregisterConfirm(root.torrent.hash,removeStrip)
    }
  }
}
