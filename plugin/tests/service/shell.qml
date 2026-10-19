import QtQuick
import Quickshell
import Quickshell.Io
import "Plugin"

ShellRoot {
  id: root
  property string createdAt: String(Date.now())
  property var fakeShell: QtObject {
    function serviceFor(id) { return id === "oliver.torrents" ? service : null }
  }
  property var testSettings: ({binary: Quickshell.env("FBT_TEST_BINARY") || "fbittorrent"})
  Service { id: service; popupOpen: true; settings: root.testSettings }
  PanelWindow {
    id: firstWindow
    visible: false
    screen: Quickshell.screens[0]
    implicitWidth: 32; implicitHeight: 32
    BarWidget { id: first; shell: root.fakeShell; settings: root.testSettings }
  }
  PanelWindow {
    id: secondWindow
    visible: false
    screen: Quickshell.screens.length > 1 ? Quickshell.screens[1] : Quickshell.screens[0]
    implicitWidth: 32; implicitHeight: 32
    BarWidget { id: second; shell: root.fakeShell; settings: root.testSettings }
  }
  IpcHandler {
    target: "fbt-test"
    function state(): string { return service.stateJson() }
    function backend(): string { return service.backendState }
    function generation(): string { return root.createdAt }
    function mode(): string { return service.client ? service.client.mode : "loading" }
    function screens(): string { return JSON.stringify([firstWindow.screen.name,secondWindow.screen.name]) }
    function widgets(): int { return service.widgets.length }
    function refresh(): void { service.refresh() }
    function start(): void { service.startDaemon() }
    function pause(hash: string): void { service.pause(hash) }
    function error(): string { return service.lastError }
    function stop(): void { service.quit() }
    function resubscribe(): void { service.client.subscribeAll() }
    function runtime(path: string): string { return service.useRuntimeDir(path) }
    function reload(): void { Quickshell.reload(true) }
  }
}
