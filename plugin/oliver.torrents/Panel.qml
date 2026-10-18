import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Ui
import qs.Ui as Ui
import "Model.js" as Model

Panel {
  id: root
  moduleName: "oliver.torrents"
  ipcTarget: "oliver.torrents"
  manageIpc: false

  readonly property color foreground: bar ? bar.foreground : Color.foreground
  readonly property color urgent: bar ? bar.urgent : Color.urgent
  readonly property color dim: Qt.darker(foreground, 1.55)
  readonly property string fontFamily: bar ? bar.fontFamily : Style.font.family
  readonly property color barIconColor: svc.online && (svc.activeTorrents.length > 0 || svc.transferring) ? barForeground : Qt.darker(barForeground, 1.55)
  readonly property string tuiCommand: String(setting("tuiCommand", "omarchy-launch-or-focus-tui --app-id=org.omarchy.fbittorrent fbittorrent tui"))

  // Cursor model: one highlight across header actions and torrent rows.
  property bool cursorActive: false
  property string focusSection: "rows"   // "header" | "rows"
  property int headerIndex: 0
  property int rowIndex: 0
  readonly property int headerCount: 4
  // The widget only exists while the backend runs: launching fbittorrent brings
  // it up, stopping the backend makes it leave the bar again.
  visible: svc.online
  readonly property var rows: svc.activeTorrents.concat(svc.otherTorrents)
  property var pendingRemoval: null

  function requestRemoval(t) {
    if (!t || svc.busy || pendingRemoval) return
    pendingRemoval = { hash: t.hash, name: t.name }
    Qt.callLater(function() { cancelRemoval.forceActiveFocus() })
  }
  function cancelRemove() {
    pendingRemoval = null
    keyCatcher.forceActiveFocus()
  }
  function confirmRemove() {
    var target = pendingRemoval
    if (!target || svc.busy) return
    cancelRemove()
    svc.removeTorrent(target)
  }

  function ensureCursor() {
    if (rows.length === 0) { focusSection = "header"; rowIndex = 0; return }
    if (rowIndex >= rows.length) rowIndex = rows.length - 1
    if (rowIndex < 0) rowIndex = 0
  }
  function moveCursor(dx, dy) {
    cursorActive = true
    ensureCursor()
    if (focusSection === "header") {
      if (dx !== 0) headerIndex = Math.max(0, Math.min(headerCount - 1, headerIndex + dx))
      if (dy > 0 && rows.length > 0) { focusSection = "rows"; rowIndex = 0; scrollCursorIntoView() }
      return
    }
    if (dy < 0 && rowIndex === 0) { setHeaderCursor(headerIndex); return }
    if (dy !== 0) { rowIndex = Math.max(0, Math.min(rows.length - 1, rowIndex + dy)); scrollCursorIntoView() }
  }
  function setHeaderCursor(i) {
    cursorActive = true
    focusSection = "header"
    headerIndex = i
    if (panelFlick) panelFlick.contentY = 0
  }
  function setRowCursor(i) {
    cursorActive = true
    focusSection = "rows"
    rowIndex = i
    scrollCursorIntoView()
  }
  function selectedTorrent() {
    if (rows.length === 0) return null
    return rows[Math.max(0, Math.min(rowIndex, rows.length - 1))]
  }
  function activateHeader() {
    if (headerIndex === 0) svc.addFromClipboard()
    else if (headerIndex === 1) openTui()
    else if (headerIndex === 2) svc.refresh()
    else { svc.stopBackend(); root.close() }
  }
  function activateCursor() {
    ensureCursor()
    if (focusSection === "header") activateHeader()
    else svc.openTorrent(selectedTorrent())
  }
  function openTui() {
    if (root.bar) root.bar.run(tuiCommand)
    root.close()
  }
  function scrollCursorIntoView() {
    if (focusSection !== "rows" || !rowColumn) return
    var item = rowColumn.children[rowIndex]
    if (!panelFlick || !item) return
    Qt.callLater(function() {
      if (!item) return
      var margin = Style.space(6)
      var p = item.mapToItem(panelFlick.contentItem, 0, 0)
      var top = p.y, bottom = top + item.height
      var viewTop = panelFlick.contentY, viewBottom = viewTop + panelFlick.height
      var maxY = Math.max(0, panelFlick.contentHeight - panelFlick.height)
      if (top < viewTop + margin) panelFlick.contentY = Math.max(0, top - margin)
      else if (bottom > viewBottom - margin) panelFlick.contentY = Math.min(maxY, bottom + margin - panelFlick.height)
    })
  }

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  onOpenedChanged: {
    pendingRemoval = null
    svc.opened = opened
    if (opened) {
      cursorActive = false
      focusSection = rows.length > 0 ? "rows" : "header"
      rowIndex = 0
      headerIndex = 0
      if (panelFlick) panelFlick.contentY = 0
      svc.refresh()
      Qt.callLater(function() { keyCatcher.forceActiveFocus() })
    }
  }
  onRowsChanged: {
    if (pendingRemoval && !rows.some(function(t) { return t.hash === pendingRemoval.hash })) cancelRemove()
    // Rows usually land a moment after the popup opens; until the user has moved
    // the cursor, keep it parked on the list so the first j goes down the torrents.
    if (!cursorActive && rows.length > 0 && focusSection === "header") focusSection = "rows"
    ensureCursor()
  }

  Service {
    id: svc
    settings: root.settings
  }

  IpcHandler {
    target: root.ipcTarget
    function open(): void { root.open() }
    function close(): void { root.close() }
    function show(): void { root.open() }
    function hide(): void { root.close() }
    function toggle(): void { root.toggle() }
    function refresh(): string { svc.refresh(); return "ok" }
    function addClipboard(): string { svc.addFromClipboard(); return "ok" }
    function tui(): string { root.openTui(); return "ok" }
    // Test hook: where the keyboard cursor is. Lets a script verify key handling
    // without screenshots (see agent_ergonomics_audit/audit/regression_tests/widget_keys.sh).
    function cursor(): string {
      return JSON.stringify({ section: root.focusSection, row: root.rowIndex, header: root.headerIndex, active: root.cursorActive, rows: root.rows.length, opened: root.opened,
        hashes: root.rows.map(function(t) { return t.hash }), confirming: !!root.pendingRemoval, confirmFocused: cancelRemoval.activeFocus ? "cancel" : "other", pendingHash: root.pendingRemoval ? root.pendingRemoval.hash : "" })
    }
  }

  BarIconButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    text: svc.online && svc.activeTorrents.length > 0 ? "󰇚" : "󰕒"
    foreground: root.barIconColor
    tooltipText: svc.online ? Model.heroMeta(svc.status) : "fbittorrent is closed"
    onPressed: function(buttonCode) {
      if (buttonCode === Qt.RightButton) svc.addFromClipboard()
      else if (buttonCode === Qt.MiddleButton) root.openTui()
      else root.toggle()
    }
  }

  KeyboardPanel {
    id: panel
    anchorItem: button
    owner: root
    bar: root.bar
    open: root.opened
    focusTarget: keyCatcher
    contentWidth: panel.fittedContentWidth(Style.space(400))
    contentHeight: panel.fittedContentHeight(column.implicitHeight, Style.space(560))

    PanelKeyCatcher {
      id: keyCatcher
      anchors.fill: parent
      onMoveRequested: function(dx, dy) {
        if (root.pendingRemoval) return
        if (!root.cursorActive) { root.cursorActive = true; return }
        root.moveCursor(dx, dy)
      }
      onActivateRequested: { if (root.pendingRemoval) root.cancelRemove(); else if (root.cursorActive) root.activateCursor() }
      onCloseRequested: { if (root.pendingRemoval) root.cancelRemove(); else root.close() }
      onTabRequested: function(direction) { if (!root.pendingRemoval) root.switchPanel(direction) }
      onTextKey: function(t) {
        if (root.pendingRemoval) { if (t === "y" || t === "Y") root.confirmRemove(); return }
        if (t === "a" || t === "A") svc.addFromClipboard()
        else if (t === "t" || t === "T") root.openTui()
        else if (t === "r" || t === "R") svc.refresh()
        else if (t === "q" || t === "Q") { svc.stopBackend(); root.close() }
        else if (t === "p" || t === "P" || t === " ") { if (root.focusSection === "rows") svc.toggleTorrent(root.selectedTorrent()) }
        else if (t === "d" || t === "D") { if (root.focusSection === "rows") root.requestRemoval(root.selectedTorrent()) }
        else if (t === "u" || t === "U") { if (root.focusSection === "rows") svc.toggleSeeding(root.selectedTorrent()) }
        else if (t === "o" || t === "O") { if (root.focusSection === "rows") svc.openTorrent(root.selectedTorrent()) }
      }

      Flickable {
        id: panelFlick
        anchors.fill: parent
        contentWidth: width
        contentHeight: column.implicitHeight
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        flickableDirection: Flickable.VerticalFlick
        interactive: contentHeight > height
        ScrollBar.vertical: ScrollBar { policy: ScrollBar.AsNeeded }

        Column {
          id: column
          width: panelFlick.width
          spacing: Style.space(12)

          PanelHero {
            id: hero
            width: parent.width
            title: "Torrents"
            meta: Model.heroMeta(svc.status)
            detail: Model.detailPill(svc.status)
            foreground: root.foreground
            fontFamily: root.fontFamily
            iconOpacity: svc.online ? 1.0 : 0.5
            iconComponent: Component {
              Text {
                textFormat: Text.PlainText
                text: svc.activeTorrents.length > 0 ? "󰇚" : "󰕒"
                color: root.foreground
                font.family: root.fontFamily
                font.pixelSize: Style.font.display
              }
            }
            trailingControl: Component {
              Row {
                spacing: Style.space(4)
                HeaderAction { index: 0; iconText: "󰐕"; tooltipText: "Add magnet from clipboard (a)"; onClicked: svc.addFromClipboard() }
                HeaderAction { index: 1; iconText: "󰆍"; tooltipText: "Open fbittorrent TUI (t)"; onClicked: root.openTui() }
                HeaderAction { index: 2; iconText: "󰑐"; tooltipText: "Refresh (r)"; onClicked: svc.refresh() }
                HeaderAction { index: 3; iconText: "󰐥"; tooltipText: "Close fbittorrent (q)"; onClicked: { svc.stopBackend(); root.close() } }
              }
            }
          }

          Column {
            visible: !!root.pendingRemoval
            width: parent.width
            spacing: Style.space(6)
            Text {
              objectName: "removal-question"
              textFormat: Text.PlainText
              width: parent.width
              text: root.pendingRemoval ? "Remove '" + root.pendingRemoval.name + "'? Files are kept." : ""
              color: root.foreground
              font.family: root.fontFamily
              font.pixelSize: Style.font.body
              wrapMode: Text.WordWrap
            }
            Row {
              spacing: Style.space(6)
              Ui.Button {
                id: cancelRemoval
                objectName: "cancel-removal"
                text: "Cancel"
                focusable: true
                bordered: true
                onClicked: root.cancelRemove()
              }
              Ui.Button {
                objectName: "confirm-removal"
                text: "Remove"
                bordered: true
                enabled: !svc.busy
                onClicked: root.confirmRemove()
              }
            }
          }

          Text {
            id: actionText
            textFormat: Text.PlainText
            visible: svc.actionStatus !== "" || svc.lastError !== ""
            width: parent.width
            text: svc.actionStatus !== "" ? svc.actionStatus : svc.lastError
            color: svc.lastError !== "" && svc.actionStatus === "" ? root.urgent : root.dim
            font.family: root.fontFamily
            font.pixelSize: Style.font.bodySmall
            wrapMode: Text.WordWrap
            elide: Text.ElideRight
            maximumLineCount: 2
            HoverHandler { id: actionHover }
            FullTextTip {
              parent: actionText
              visible: actionHover.hovered && actionText.truncated
              text: actionText.text
            }
          }

          Text {
            textFormat: Text.PlainText
            visible: !svc.online
            width: parent.width
            text: "fbittorrent is closed."
            color: root.dim
            font.family: root.fontFamily
            font.pixelSize: Style.font.body
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
          }

          Text {
            textFormat: Text.PlainText
            visible: svc.online && root.rows.length === 0
            width: parent.width
            text: "Nothing here yet.\nCopy a magnet link and press a."
            color: root.dim
            font.family: root.fontFamily
            font.pixelSize: Style.font.body
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
          }

          PanelSeparator {
            visible: svc.online && root.rows.length > 0
            foreground: root.foreground
          }

          Column {
            id: rowColumn
            objectName: "torrent-rows"
            visible: svc.online && root.rows.length > 0
            width: parent.width
            spacing: Style.space(6)

            Repeater {
              model: root.rows
              TorrentRow {
                required property var modelData
                required property int index
                width: rowColumn.width
                torrent: modelData
                rowIdx: index
              }
            }
          }
        }
      }
    }
  }

  component HeaderAction: PanelActionButton {
    property int index: 0
    foreground: root.foreground
    fontFamily: root.fontFamily
    bordered: true
    hasCursor: root.cursorActive && root.focusSection === "header" && root.headerIndex === index
    onHovered: function(on) { if (on) root.setHeaderCursor(index) }
  }

  component FullTextTip: PanelToolTip {
    id: tip
    delay: 500
    width: panelFlick.width
    x: 0
    y: parent ? parent.height + Style.space(6) : 0
    fontFamily: root.fontFamily
    contentItem: Text {
      textFormat: Text.PlainText
      text: tip.text
      color: tip.panelForeground
      font.family: tip.fontFamily
      font.pixelSize: tip.fontSize
      wrapMode: Text.Wrap
      leftPadding: Border.left(tip.panelBorderSpec) + Style.spacing.controlPaddingX
      rightPadding: Border.right(tip.panelBorderSpec) + Style.spacing.controlPaddingX
      topPadding: Border.top(tip.panelBorderSpec) + Style.spacing.controlPaddingY
      bottomPadding: Border.bottom(tip.panelBorderSpec) + Style.spacing.controlPaddingY
    }
  }

  component TorrentRow: CursorSurface {
    id: row
    property var torrent: null
    property int rowIdx: 0
    readonly property bool active: torrent ? Model.isActive(torrent) : false
    readonly property bool stopped: torrent ? Model.isStopped(torrent) : false
    readonly property real progress: torrent ? Math.max(0, Math.min(1, Number(torrent.progress))) : 0

    hasCursor: root.cursorActive && root.focusSection === "rows" && root.rowIndex === rowIdx
    foreground: root.foreground
    implicitHeight: content.implicitHeight + Style.spacing.rowPaddingX

    MouseArea {
      anchors.fill: parent
      hoverEnabled: true
      cursorShape: Qt.PointingHandCursor
      acceptedButtons: Qt.LeftButton | Qt.MiddleButton
      onEntered: root.setRowCursor(row.rowIdx)
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
        color: row.active ? root.foreground : root.dim
        font.family: root.fontFamily
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
          color: row.stopped ? root.dim : root.foreground
          font.family: root.fontFamily
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
          visible: row.progress < 1 || row.active
          Rectangle {
            anchors.fill: parent
            radius: height / 2
            color: Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.12)
          }
          Rectangle {
            width: parent.width * row.progress
            height: parent.height
            radius: height / 2
            color: row.active ? root.foreground : root.dim
            Behavior on width { NumberAnimation { duration: 400; easing.type: Easing.OutCubic } }
          }
        }

        Text {
          id: metaText
          textFormat: Text.PlainText
          Layout.fillWidth: true
          text: row.torrent ? Model.stateLabel(row.torrent) + " · " + Model.meta(row.torrent) : ""
          color: root.dim
          font.family: root.fontFamily
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
        iconText: row.stopped ? "󰐊" : "󰏤"
        tooltipText: row.stopped ? "Start (p)" : "Stop (p)"
        foreground: root.foreground
        fontFamily: root.fontFamily
        enabled: !svc.busy
        Layout.alignment: Qt.AlignVCenter
        onClicked: svc.toggleTorrent(row.torrent)
      }
      PanelActionButton {
        iconText: "󰝰"
        tooltipText: "Open the file, or the folder (o)"
        foreground: root.foreground
        fontFamily: root.fontFamily
        enabled: !svc.busy
        Layout.alignment: Qt.AlignVCenter
        onClicked: svc.openTorrent(row.torrent)
      }
      PanelActionButton {
        objectName: "trash-" + (row.torrent ? row.torrent.hash : "")
        iconText: "󰆴"
        tooltipText: "Remove torrent, keep files (d)"
        foreground: root.foreground
        hoverColor: root.urgent
        fontFamily: root.fontFamily
        enabled: !svc.busy
        Layout.alignment: Qt.AlignVCenter
        onClicked: root.requestRemoval(row.torrent)
      }
    }
  }
}
