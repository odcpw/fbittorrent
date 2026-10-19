import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Commons as Commons
import "components"
import "components" as Components
import qs.Ui
import qs.Ui as Ui
import "Model.js" as Model
import "Glyphs.js" as Glyphs
import "Geometry.js" as Geometry

Ui.Panel {
  id: root
  moduleName: "oliver.torrents"
  ipcTarget: "oliver.torrents"
  manageIpc: false

  property var shell: bar && bar.shell ? bar.shell : null
  readonly property var svc: shell ? shell.serviceFor("oliver.torrents") : null
  readonly property color foreground: bar ? bar.foreground : Commons.Color.foreground
  readonly property color urgent: bar ? bar.urgent : Commons.Color.urgent
  readonly property color dim: Model.dim(foreground,Commons.Color.popups.background)
  readonly property string fontFamily: bar ? bar.fontFamily : Style.font.family
  readonly property color barIconColor: svc && svc.online && (svc.activeTorrents.length > 0 || svc.transferring) ? barForeground : Qt.darker(barForeground, 1.55)
  readonly property string tuiCommand: String(setting("tuiCommand", "omarchy-launch-or-focus-tui --app-id=org.omarchy.fbittorrent fbittorrent tui"))

  // Cursor model: one highlight across header actions and torrent rows.
  property bool cursorActive: false
  property string focusSection: "rows"   // "header" | "rows"
  property int headerIndex: 0
  property int rowIndex: 0
  readonly property int headerCount: 4
  // The widget only exists while the backend runs: launching fbittorrent brings
  // it up, stopping the backend makes it leave the bar again.
  visible: !!svc && (svc.online || setting("showWhenStopped",false) || root.opened || svc.stoppedRecently || svc.reconnecting)
  readonly property var rows: svc ? svc.torrents : []
  property var pendingRemoval: null

  function popupItem() { return keyCatcher }
  function geometry() { return root.opened ? Geometry.describe(keyCatcher,Style.font.title) : null }

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
    if (!svc) return
    pendingRemoval = null
    if (svc) svc.updatePopup()
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

  onSettingsChanged: if (svc) svc.settings = settings
  onSvcChanged: if (svc) { svc.registerWidget(root); svc.settings = root.settings }
  Component.onDestruction: if (svc) svc.unregisterWidget(root)

  Loader {
    active: !!root.svc && root.svc.ipcOwner === root
    sourceComponent: Component {

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
    function state(): string { var state=JSON.parse(svc.stateJson());state.geometry=root.geometry();return JSON.stringify(state) }
    function useRuntimeDir(path: string): string { return svc.useRuntimeDir(path) }
    function cursor(): string {
      return JSON.stringify({ section: root.focusSection, row: root.rowIndex, header: root.headerIndex, active: root.cursorActive, rows: root.rows.length, opened: root.opened,
        hashes: root.rows.map(function(t) { return t.hash }), confirming: !!root.pendingRemoval, confirmFocused: cancelRemoval.activeFocus ? "cancel" : "other", pendingHash: root.pendingRemoval ? root.pendingRemoval.hash : "" })
    }
  }
    }
  }

  function barClick(buttonCode) {
    if (!svc) return
    if (buttonCode===Qt.RightButton) { root.open(); svc.addFromClipboard() }
    else if (buttonCode===Qt.MiddleButton) {
      if (root.setting("middleClick","dashboard")==="tui") root.openTui()
      else svc.openDashboard("torrents")
    } else {
      if (!svc.online && root.setting("showWhenStopped",false)) {svc.startDaemon();root.open()}
      else root.toggle()
    }
  }
  Components.BarIndicator {
    id: button
    anchors.fill: parent
    bar: root.bar
    service: svc
    showRates: !!root.setting("showRates",false)
    onPressed: function(buttonCode) { root.barClick(buttonCode) }
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
      onActivateRequested: { if (svc.clipboardOffer) svc.acceptOffer(); else if (root.pendingRemoval) root.cancelRemove(); else if (root.cursorActive) root.activateCursor() }
      onCloseRequested: { if (svc.clipboardOffer) svc.cancelOffer(); else if (root.pendingRemoval) root.cancelRemove(); else root.close() }
      onTabRequested: function(direction) { if (!root.pendingRemoval) root.switchPanel(direction) }
      onTextKey: function(t) {
        if (svc.clipboardOffer) { if (t==="y" || t==="Y") svc.acceptOffer(); return }
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
            meta: Model.heroMeta(svc ? svc.status : {ok:false})
            detail: Model.detailPill(svc ? svc.status : {ok:false})
            foreground: root.foreground
            fontFamily: root.fontFamily
            iconOpacity: svc && svc.online ? 1.0 : 0.5
            iconComponent: Component {
              Text {
                textFormat: Text.PlainText
                text: svc && svc.activeTorrents.length > 0 ? Glyphs.download : Glyphs.upload
                color: root.foreground
                font.family: root.fontFamily
                font.pixelSize: Style.font.display
              }
            }
            trailingControl: Component {
              Row {
                spacing: Style.space(4)
                HeaderAction { host: root; index: 0; iconText: Glyphs.add; tooltipText: "Add magnet from clipboard (a)"; onClicked: svc.addFromClipboard() }
                HeaderAction { host: root; index: 1; iconText: Glyphs.terminal; tooltipText: "Open fbittorrent TUI (t)"; onClicked: root.openTui() }
                HeaderAction { host: root; index: 2; iconText: Glyphs.refresh; tooltipText: "Refresh (r)"; onClicked: svc.refresh() }
                HeaderAction { host: root; index: 3; iconText: Glyphs.power; tooltipText: "Close fbittorrent (q)"; onClicked: { svc.stopBackend(); root.close() } }
              }
            }
          }

          ClipboardOffer { width:parent.width; service:root.svc }

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
                enabled: !!svc && !svc.busy
                onClicked: root.confirmRemove()
              }
            }
          }

          Text {
            id: actionText
            textFormat: Text.PlainText
            visible: !!svc && (svc.actionStatus !== "" || svc.lastError !== "")
            width: parent.width
            text: svc ? (svc.actionStatus !== "" ? svc.actionStatus : svc.lastError) : ""
            color: svc && svc.lastError !== "" && svc.actionStatus === "" ? root.urgent : root.dim
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
            visible: !svc || !svc.online
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
            visible: svc && svc.online && root.rows.length === 0
            width: parent.width
            text: svc && svc.status.loaded === false ? "Loading torrents…" : "Nothing here yet.\nCopy a magnet link and press a."
            color: root.dim
            font.family: root.fontFamily
            font.pixelSize: Style.font.body
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
          }

          PanelSeparator {
            visible: svc && svc.online && root.rows.length > 0
            foreground: root.foreground
          }

          Column {
            id: rowColumn
            objectName: "torrent-rows"
            visible: svc && svc.online && root.rows.length > 0
            width: parent.width
            spacing: Style.space(6)

            Repeater {
              model: svc ? svc.torrentModel : null
              TorrentRow {
                host: root
                required property var model
                required property int index
                width: rowColumn.width
                torrent: model
                rowIdx: index
              }
            }
          }
        }
      }
    }
  }


}
