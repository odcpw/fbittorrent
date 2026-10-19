import QtQuick
import Quickshell
import Quickshell.Io
import "Plugin"

ShellRoot {
  id: root
  property string createdAt: String(Date.now())
  property string summoned: ""
  property var fakeShell: QtObject {
    function summon(id, args) { root.summoned=id+":"+args; return true }
    function serviceFor(id) { return id === "oliver.torrents" ? service : null }
  }
  property var testSettings: ({binary: Quickshell.env("FBT_TEST_BINARY") || "fbittorrent"})
  Service { id: service; shell:root.fakeShell; popupOpen: true; settings: root.testSettings }
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
    function summon(): string { return root.summoned }
    function middle(): void { first.barClick(Qt.MiddleButton) }
    function offer(): string { return JSON.stringify(service.clipboardOffer) }
    function clipboard(): void { service.addFromClipboard() }
    function accept(): void { service.acceptOffer() }
    function cancel(): void { service.cancelOffer() }
    function recent(): bool { return service.stoppedRecently }
    function shown(): bool { return first.visible }
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
