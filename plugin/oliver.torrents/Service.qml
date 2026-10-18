import QtQuick
import Quickshell
import Quickshell.Io
import "Model.js" as Model

// Talks to the open app through the `fbittorrent` CLI. One JSON poll per tick; the
// panel derives everything else from `status`.
Item {
  id: root

  property var settings: ({})
  property bool opened: false

  property var status: ({ ok: false, dl: 0, up: 0, active: 0, seeding: 0, stopped: 0, total: 0, torrents: [] })
  readonly property bool online: status.ok === true
  readonly property string backendState: online ? "running" : String(status.backend || "stopped")
  readonly property var torrents: status.torrents || []
  readonly property var activeTorrents: torrents.filter(Model.isActive)
  readonly property var otherTorrents: torrents.filter(function(t) { return !Model.isActive(t) })
  readonly property bool transferring: online && (Number(status.dl) > 0 || Number(status.up) > 0)
  property string lastError: ""
  property string actionStatus: ""
  readonly property bool busy: actionProc.running

  readonly property string bin: String(setting("binary", "fbittorrent"))
  readonly property int refreshMs: 1000 * intSetting("refreshIntervalSec", 2, 1, 60)
  readonly property int idleRefreshMs: 1000 * intSetting("idleRefreshIntervalSec", 10, 2, 600)

  function setting(name, fallback) {
    var v = settings ? settings[name] : undefined
    return v === undefined || v === null ? fallback : v
  }
  function intSetting(name, fallback, min, max) {
    var n = parseInt(String(setting(name, fallback)), 10)
    if (!isFinite(n)) n = fallback
    return Math.max(min, Math.min(max, n))
  }

  function refresh() {
    if (statusProc.running) return
    statusProc.running = true
  }

  // Serialised fire-and-forget actions. Each refreshes the list when done.
  property var _queue: []
  function runAction(args, label) {
    _queue.push({ args: args, label: label })
    pump()
  }
  function pump() {
    if (actionProc.running || _queue.length === 0) return
    var next = _queue.shift()
    actionStatus = next.label
    actionProc.command = [bin].concat(next.args)
    actionProc.running = true
  }

  function toggleTorrent(t) {
    if (!t) return
    runAction([Model.isStopped(t) ? "start" : "stop", t.hash], (Model.isStopped(t) ? "Starting " : "Stopping ") + t.name)
  }
  function removeTorrent(t) {
    if (!t) return
    runAction(["delete", t.hash], "Removing " + t.name)
  }
  function toggleSeeding(t) {
    if (!t) return
    var allow = !Model.seedAllowed(t)
    runAction([allow ? "seed" : "noseed", t.hash], (allow ? "Allowing seeding for " : "Stopping seeding for ") + t.name)
  }
  function openTorrent(t) {
    if (!t) return
    runAction(["open", t.hash], "Opening " + t.name)
  }
  function addFromClipboard() { runAction(["add-clipboard"], "Adding from clipboard…") }
  function stopBackend() { runAction(["down"], "Closing fbittorrent…") }
  function startBackend() { runAction(["up"], "Opening fbittorrent…") }
  function reconcile() { runAction(["reconcile", "--recent", "180", "--json"], "") }

  Process {
    id: statusProc
    command: [root.bin, "status", "--json"]
    stdout: StdioCollector {
      onStreamFinished: {
        var parsed = Model.parseStatus(text)
        if (parsed.ok) {
          root.status = parsed
          root.lastError = ""
        } else {
          root.status = { ok: false, backend: parsed.backend || "stopped", dl: 0, up: 0, active: 0, seeding: 0, stopped: 0, total: 0, torrents: [] }
        }
      }
    }
    stderr: StdioCollector {
      onStreamFinished: if (text.trim() !== "") root.lastError = text.trim().replace(/^fbittorrent: /, "")
    }
  }

  Process {
    id: actionProc
    stdout: StdioCollector {}
    stderr: StdioCollector {
      onStreamFinished: if (text.trim() !== "") root.lastError = text.trim().replace(/^fbittorrent: /, "")
    }
    onExited: function(code) {
      root.actionStatus = ""
      root.refresh()
      root.pump()
    }
  }

  Timer {
    interval: root.opened ? root.refreshMs : root.idleRefreshMs
    running: true
    repeat: true
    triggeredOnStart: true
    onTriggered: root.refresh()
  }

  // Catch torrents added outside fbittorrent (RSS, WebUI) and give them the default flags.
  Timer {
    interval: 60000
    running: root.online
    repeat: true
    onTriggered: root.reconcile()
  }
}
