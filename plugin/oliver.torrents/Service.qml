import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons as Commons
import "ApiClient.js" as ApiClient
import "Store.js" as Store
import "Model.js" as Model

Item {
  id: root
  property var shell: null
  property var manifest: ({version: "0.2.0"})
  property var settings: ({})
  property bool popupOpen: false
  property bool dashboardVisible: false
  property var detailTopics: []
  property string runtimeOverride: ""
  readonly property string runtimeDir: runtimeOverride || Quickshell.env("XDG_RUNTIME_DIR")
  readonly property string bin: String(settings.binary || "fbittorrent")
  readonly property bool online: status.ok === true && status.running !== false
  property bool reconnecting: false
  property bool stoppedRecently: false
  property string instance: ""
  property string backendState: "stopped"
  property string lastError: ""
  property string actionStatus: ""
  property var pending: []
  readonly property bool busy: pending.length > 0
  property var torrents: []
  property string sortKey: "added"
  property bool sortReverse: false
  readonly property alias torrentModel: rowsModel
  ListModel { id: rowsModel; dynamicRoles: true }
  function syncRows() {
    if (store.syncModel(rowsModel, sortKey, sortReverse)) torrents = store.rows(store.modelOrder())
  }
  onSortKeyChanged: syncRows()
  onSortReverseChanged: syncRows()
  readonly property var activeTorrents: torrents.filter(Model.isActive)
  readonly property var otherTorrents: torrents.filter(function(t) { return !Model.isActive(t) })
  readonly property bool transferring: Number(status.dl) > 0 || Number(status.up) > 0
  property var status: ({ok:false, torrents:[]})
  property var store: Store.create()
  property var client: null
  property var widgets: []
  property var ipcOwner: null
  function registerWidget(widget) {
    if (widgets.indexOf(widget) < 0) widgets = widgets.concat([widget])
    if (!ipcOwner) ipcOwner = widget
  }
  function unregisterWidget(widget) {
    widgets = widgets.filter(function(w) { return w !== widget })
    if (ipcOwner === widget) {
      ipcOwner = null
      ipcClaim.restart()
    }
    updatePopup()
  }
  function updatePopup() { popupOpen = widgets.some(function(w) { return w.opened }) }


  function syncClient(c) {
    if (status.ok !== c.online) {
      var connectionStatus = Object.assign({}, status)
      connectionStatus.ok = c.online; connectionStatus.running = c.online
      status = connectionStatus
    }
    reconnecting = c.reconnecting; instance = c.instance
    pending = c.state().pending
    lastError = c.lastError ? c.lastError.message : ""
    backendState = c.online && c.hello ? (store.summary.daemon ? store.summary.daemon.state : store.info.state || c.hello.state) : c.mode
  }
  function applyFrame(frame, request) {
    var rowsChanged
    try { rowsChanged = store.apply(frame, request) }
    catch (error) { client.reset("invalid state stream: " + String(error)); return }
    syncRows()
    if (rowsChanged || frame.topic === "summary" || (request && frame.ok)) {
      status = Model.apiStatus(store, online)
    }
    if (frame.ev === "bye") { stoppedRecently = true; stoppedTimer.restart() }
  }
  function mutate(op, args, cap, callback) {
    if (!client) return false
    if (cap && client.caps.indexOf(cap) < 0) {
      lastError = "This daemon does not support " + cap
      return false
    }
    return client.request(op, args, {mutating:true, callback:callback})
  }
  function refresh() { if (client) client.subscribeAll() }
  function pause(hash) { return mutate("torrents.stop", {hashes:[hash]}) }
  function resume(hash) { return mutate("torrents.start", {hashes:[hash]}) }
  function toggleSeed(hash) {
    var row = store.torrents[hash]
    return row ? mutate("torrents.set_seed", {hashes:[hash], on:!row.seed}) : false
  }
  function remove(hash, deleteFiles) { return mutate("torrents.remove", {hashes:[hash], delete_files:!!deleteFiles}) }
  function add(source, stage) { return mutate("torrents.add", {source:source, stage:!!stage}, stage ? "staging" : "") }
  function commitStaged(hash, files, seed, savePath) { return mutate("torrents.commit", {hash:hash, files:files, seed:seed, save_path:savePath}, "staging") }
  function discardStaged(hash) { return mutate("torrents.discard", {hash:hash}, "staging") }
  function openTarget(hash, file) {
    var args = {hash:hash}
    if (file !== undefined) args.file = file
    return client.request("torrents.open_target", args, {callback:function(result) {
      if (result && result.path) Commons.Util.execArgv(["xdg-open", result.path])
    }})
  }
  function setTurtle(on) { return mutate("transfer.set_alt", {on:on}, "limits") }
  function recheck(hash) { return mutate("torrents.recheck", {hashes:[hash]}, "recheck") }
  function quit() { return mutate("app.shutdown", {reason:"user"}) }
  function dismissClipboard() { return mutate("app.clipboard.dismiss", {}, "clipboard") }
  function startDaemon() { Commons.Util.execArgv([bin, "up"]) }
  function useRuntimeDir(path) {
    if (Quickshell.env("FBT_TEST_HOOKS") !== "1") return "refused: FBT_TEST_HOOKS=1 required"
    socket.connected = false
    runtimeOverride = path
    client.disconnected()
    Qt.callLater(function() { reconnect.restart() })
    return "ok"
  }
  function stateJson() { return JSON.stringify(client ? client.state() : {online:false, instance:"", subs:{}, pending:[], view:null}) }
  function toggleTorrent(t) { return t ? (Model.isStopped(t) ? resume(t.hash) : pause(t.hash)) : false }
  function removeTorrent(t) { return t ? remove(t.hash, false) : false }
  function toggleSeeding(t) { return t ? toggleSeed(t.hash) : false }
  function openTorrent(t) { return t ? openTarget(t.hash) : false }
  function stopBackend() { return quit() }
  function startBackend() { startDaemon() }
  property bool clipboardTooLarge: false
  function addFromClipboard() {
    if (!clipboard.running) { clipboardTooLarge = false; clipboard.running = true }
  }
  function clipboardSource(text) {
    var value = text.trim(), source = null, hash = ""
    if (/^magnet:\?/i.test(value)) {
      var match = /[?&]xt=urn:btih:([0-9a-f]{40})(?:&|$)/i.exec(value)
      if (match) hash = match[1].toLowerCase()
      source = {magnet:value}
    } else if (/^https?:\/\//i.test(value)) source = {url:value}
    else if (/^\//.test(value) && /\.torrent$/i.test(value)) source = {path:value}
    if (hash && store.torrents[hash]) { actionStatus = "Already in your list: " + store.torrents[hash].name; return null }
    if (!source) actionStatus = "Clipboard has no torrent"
    return source
  }
  onPopupOpenChanged: if (client) client.visibility(popupOpen, dashboardVisible, detailTopics)
  onDashboardVisibleChanged: if (client) client.visibility(popupOpen, dashboardVisible, detailTopics)
  onDetailTopicsChanged: if (client) client.visibility(popupOpen, dashboardVisible, detailTopics)

  Socket {
    id: socket
    path: root.runtimeDir + "/fbittorrent/api.sock"
    parser: SplitParser { onRead: function(data) { if (root.client) root.client.receive(data) } }
    onConnectionStateChanged: {
      if (!root.client) return
      if (connected) { reconnect.stop(); root.client.connected() }
      else { root.client.disconnected(); reconnect.restart() }
    }
    onError: if (root.client && !connected) reconnect.restart()
  }
  Timer {
    id: reconnect
    interval: root.client ? root.client.retryDelay : 500
    onTriggered: {
      if (!root.client || socket.connected) return
      root.client.attempt()
      if (!socket.connected) restart()
    }
  }
  Timer { interval: 1000; repeat: true; running: !!root.client; onTriggered: { root.client.tick(); root.syncRows() } }
  Timer { id: ipcClaim; interval: 0; onTriggered: if (!root.ipcOwner && root.widgets.length) root.ipcOwner = root.widgets[0] }
  Timer { id: stoppedTimer; interval: 10000; onTriggered: root.stoppedRecently = false }
  Process {
    id: clipboard
    command: ["wl-paste", "--no-newline"]
    stdout: StdioCollector {
      waitForEnd: false
      onDataChanged: if (ApiClient.utf8Bytes(text) > 65536) {
        root.clipboardTooLarge = true
        root.lastError = "Clipboard exceeds 65536 bytes"
        clipboard.running = false
      }
      onStreamFinished: {
        if (root.clipboardTooLarge) return
        var source = root.clipboardSource(text)
        if (source) root.add(source, false)
      }
    }
    onExited: function(code) { if (code !== 0) root.lastError = "Could not read clipboard" }
  }
  Component.onCompleted: {
    client = ApiClient.create({
      write:function(text) { socket.write(text) }, flush:function() { socket.flush() },
      isConnected:function() { return socket.connected }, connect:function() { socket.connected = true },
      disconnect:function() { socket.connected = false }
    }, {version:manifest.version || "0.2.0", changed:syncClient, frame:applyFrame,
      instance:function() { root.store = Store.create(); root.torrents = [] }})
    if (socket.connected) client.connected()
    else { client.attempt(); if (socket.connected) client.connected() }
    if (!socket.connected) reconnect.start()
  }
}
