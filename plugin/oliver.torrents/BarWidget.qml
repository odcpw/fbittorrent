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
  readonly property string tuiCommand: String(setting("tuiCommand", "omarchy-launch-or-focus-tui --app-id=org.omarchy.fbittorrent fbittorrent tui"))

  visible: !!svc && (svc.online || setting("showWhenStopped",false) || root.opened || svc.stoppedRecently || svc.reconnecting)
  function popupItem() { return panel.loadedContent }
  function geometry() { return root.opened && popupItem() ? Geometry.describe(popupItem(),Style.font.title) : null }
  function openTui() { if(root.bar)root.bar.run(tuiCommand);root.close() }
  onOpenedChanged: if(svc)svc.updatePopup()
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
    function state(): string { var state=JSON.parse(svc.stateJson());state.geometry=root.geometry();state.popupDelegates=root.popupItem() ? root.popupItem().delegateCount : 0;return JSON.stringify(state) }
    function summonView(payload: string): string {
      if(Quickshell.env("FBT_TEST_HOOKS")!=="1")return "refused: FBT_TEST_HOOKS=1 required"
      return root.shell && root.shell.summon("oliver.torrents",payload) ? "ok" : "unavailable"
    }
    function scroll(dy: real): string {
      if(Quickshell.env("FBT_TEST_HOOKS")!=="1")return "refused: FBT_TEST_HOOKS=1 required"
      if(!svc.dashboardOwner)return "unavailable"
      svc.dashboardOwner.scroll(Math.max(-10000,Math.min(10000,dy)));return "ok"
    }
    function useRuntimeDir(path: string): string { return svc.useRuntimeDir(path) }
    function cursor(): string {
      var c=root.popupItem() ? root.popupItem().cursor() : {rows:0,hashes:[],hints:false}
      c.opened=root.opened;return JSON.stringify(c)
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
    id:panel
    anchorItem:button;owner:root;bar:root.bar;open:root.opened
    focusTarget:panel.loadedContent
    contentWidth:panel.fittedContentWidth(Style.space(380))
    contentHeight:panel.fittedContentHeight(panel.loadedContentHeight,Style.space(560))
    content:Component {Components.PopupContent {owner:root}}
  }
}
