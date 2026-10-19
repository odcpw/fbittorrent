import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import qs.Commons
import qs.Commons as Commons
import qs.Ui
import qs.Ui as Ui
import "../Model.js" as Model

CursorSurface {
  property var host
  readonly property var svc: host.svc

    id: row
    property var torrent: null
    property int rowIdx: 0
    readonly property bool active: torrent ? Model.isActive(torrent) : false
    readonly property bool stopped: torrent ? Model.isStopped(torrent) : false
    readonly property real progress: torrent && torrent.progress !== undefined && torrent.progress !== null ? Math.max(0, Math.min(1, Number(torrent.progress))) : 0

    hasCursor: host.cursorActive && host.focusSection === "rows" && host.rowIndex === rowIdx
    foreground: host.foreground
    implicitHeight: content.implicitHeight + Style.spacing.rowPaddingX

    MouseArea {
      anchors.fill: parent
      hoverEnabled: true
      cursorShape: Qt.PointingHandCursor
      acceptedButtons: Qt.LeftButton | Qt.MiddleButton
      onEntered: host.setRowCursor(row.rowIdx)
      onClicked: function(mouse) {
        if (mouse.button === Qt.MiddleButton) svc.toggleTorrent(row.torrent)
        else svc.openTorrent(row.torrent)
      }
    }

    RowLayout {
      id: content
      anchors.left: parent.left
      anchors.right: parent.right
      anchors.verticalCenter: parent.verticalCenter
      anchors.leftMargin: Style.space(10)
      anchors.rightMargin: Style.space(6)
      spacing: Style.space(8)

      Text {
        textFormat: Text.PlainText
        text: row.torrent ? Model.glyph(row.torrent) : ""
        color: row.active ? host.foreground : host.dim
        font.family: host.fontFamily
        font.pixelSize: Style.font.icon
        Layout.alignment: Qt.AlignVCenter
      }

      ColumnLayout {
        Layout.fillWidth: true
        spacing: Style.space(3)

        Text {
          id: nameText
          textFormat: Text.PlainText
          Layout.fillWidth: true
          text: row.torrent ? row.torrent.name : ""
          color: row.stopped ? host.dim : host.foreground
          font.family: host.fontFamily
          font.pixelSize: Style.font.body
          elide: Text.ElideMiddle
          HoverHandler { id: nameHover }
          FullTextTip {
            parent: nameText
            visible: nameHover.hovered && nameText.truncated
            text: nameText.text + "\n" + metaText.text
          }
        }

        // Thin progress rule, only while there is progress worth showing.
        Item {
          Layout.fillWidth: true
          implicitHeight: Style.space(3)
          visible: row.torrent && row.torrent.progress !== undefined && row.torrent.progress !== null && (row.progress < 1 || row.active)
          Rectangle {
            anchors.fill: parent
            radius: height / 2
            color: Qt.rgba(host.foreground.r, host.foreground.g, host.foreground.b, 0.12)
          }
          Rectangle {
            width: parent.width * row.progress
            height: parent.height
            radius: height / 2
            color: row.active ? host.foreground : host.dim
            Behavior on width { NumberAnimation { duration: 400; easing.type: Easing.OutCubic } }
          }
        }

        Text {
          id: metaText
          textFormat: Text.PlainText
          Layout.fillWidth: true
          text: row.torrent ? Model.stateLabel(row.torrent) + " · " + Model.meta(row.torrent) : ""
          color: host.dim
          font.family: host.fontFamily
          font.pixelSize: Style.font.caption
          elide: Text.ElideRight
          HoverHandler { id: metaHover }
          FullTextTip {
            parent: metaText
            visible: metaHover.hovered && metaText.truncated
            text: nameText.text + "\n" + metaText.text
          }
        }
      }

      PanelActionButton {
        objectName: "toggle-" + (row.torrent ? row.torrent.hash : "")
        visible: !(row.torrent && (row.torrent.intent !== undefined
                   ? row.torrent.state === "done" && !row.torrent.seed
                   : Number(row.torrent.completion_on || 0) > 0 && Number(row.torrent.ratio_limit) === 0))
        iconText: row.stopped ? "󰐊" : "󰏤"
        tooltipText: row.stopped ? "Start (p)" : "Stop (p)"
        foreground: host.foreground
        fontFamily: host.fontFamily
        enabled: !svc.busy
        Layout.alignment: Qt.AlignVCenter
        onClicked: svc.toggleTorrent(row.torrent)
      }
      PanelActionButton {
        iconText: "󰝰"
        tooltipText: "Open the file, or the folder (o)"
        foreground: host.foreground
        fontFamily: host.fontFamily
        enabled: !svc.busy
        Layout.alignment: Qt.AlignVCenter
        onClicked: svc.openTorrent(row.torrent)
      }
      PanelActionButton {
        objectName: "trash-" + (row.torrent ? row.torrent.hash : "")
        iconText: "󰆴"
        tooltipText: "Remove torrent, keep files (d)"
        foreground: host.foreground
        hoverColor: host.urgent
        fontFamily: host.fontFamily
        enabled: !svc.busy
        Layout.alignment: Qt.AlignVCenter
        onClicked: host.requestRemoval(row.torrent)
      }
    }
  }
