import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell
import qs.Commons as Commons
import qs.Ui as Ui
import "Model.js" as Model
import "PopupState.js" as PopupState
import "Glyphs.js" as Glyphs
import "AddSource.js" as Source
import "views" as Views
import "components" as Components

Item {
  id:root
  property var shell:null
  property var manifest:({})
  property var service:null
  readonly property var svc:service || (shell ? shell.serviceFor("oliver.torrents") : null)
  property string currentView:"torrents"
  property string settingsReturnView:"torrents"
  property bool helpOpen:false
  readonly property var windowContent:scope
  readonly property var settingsSheet:settingsLoader.item
  readonly property var logView:logLoader.item
  readonly property var helpOverlay:helpLoader.item
  property string query:""
  property string filter:"all"
  property string selectedHash:""
  property string tableQuery:""
  property bool torrentsVisited:false
  property bool closingFromHost:false
  property bool disposing:false
  property string routeError:""
  readonly property var torrents:torrentsLoader.item
  readonly property bool torrentsEditing:currentView==="torrents" && !!torrents && torrents.editing
  readonly property bool torrentsDialogOpen:currentView==="torrents" && !!torrents && torrents.dialogOpen
  readonly property var search:searchLoader.item
  readonly property var shows:showsLoader.item
  readonly property var rss:rssLoader.item
  readonly property var views:["torrents","search","shows","rss","settings","log"]
  readonly property var labels:["Torrents","Search","Shows","RSS","Settings","Log"]
  function selectTorrent(hash){currentView="torrents";selectedHash=hash;if(torrents)torrents.cursorHash=hash}
  function focusAfterAdd(){
    if(currentView==="search" && search){search.focusResults();svc.focusWindow(window,search)}else focusView()
  }
  function openAdd(source,options){var accepted=svc ? svc.beginAdd(source,options) : false;Qt.callLater(function(){if(addSheet.item)addSheet.item.focusSheet()});return accepted}
  function acceptDrop(urls,text){var parsed=Source.drop(urls,text);if(!parsed)return false;return openAdd(parsed.source)}
  function publish() {
    if(disposing)return
    if(svc && !svc.testHooks && svc.dashboardOwner===(window.visible ? root : null) && svc.dashboardVisible===window.visible)return
    if(!window || !window.visible){publishNow();return}
    // Loader items and their derived view state settle after onLoaded. Publish
    // once per event turn, rather than allocating intermediate partial states.
    Qt.callLater(root.publishNow)
  }
  function publishNow(inspect) {
    if (disposing || !svc || !window)return
    if(!window.visible && svc.dashboardOwner!==root)return
    svc.dashboardOwner=window.visible ? root : null
    svc.dashboardVisible=window.visible
    if(!window.visible)svc.dashboardState=null
    if(!svc.testHooks || inspect!==true)return
    svc.dashboardState=window.visible ? {window:{visible:true,width:window.width,height:window.height,view:currentView},
      clipboard:svc.clipboardOffer ? {hash:svc.clipboardOffer.hash,name:svc.clipboardOffer.name} : null,diskBanner:svc.diskBanner,turtle:svc.turtleOn,cursor:"view",selection:selectedHash ? [selectedHash] : [],filter:filter,query:query} : null
    if(window.visible && currentView==="torrents" && torrents)svc.dashboardState=Object.assign({},svc.dashboardState,torrents.snapshot())
    if(window.visible && currentView==="search" && search)svc.dashboardState=Object.assign({},svc.dashboardState,search.snapshot())
    if(window.visible && currentView==="rss" && rss)svc.dashboardState=Object.assign({},svc.dashboardState,rss.snapshot())
    if(window.visible && currentView==="shows" && shows)svc.dashboardState=Object.assign({},svc.dashboardState,shows.snapshot())
    if(window.visible && settingsSheet)svc.dashboardState.settings=settingsSheet.state
    if(window.visible && logView)svc.dashboardState.log={level:logView.level,source:logView.source,hash:logView.hashFilter,count:logView.count,follow:logView.follow}
    if(window.visible){svc.dashboardState.help=helpOpen;svc.dashboardState.stopping=stopConfirm.opened;svc.dashboardState.stopSelected=stopConfirm.selectedIndex}
    if(window.visible && svc.addFlow){svc.dashboardState.add=svc.addFlow.snapshot();svc.dashboardState.add.picker=addSheet.item ? addSheet.item.pickerState() : null;svc.dashboardState.add.field=addSheet.item ? addSheet.item.field() : "";if(svc.dashboardState.add.visible){svc.dashboardState.cursor="add"}}

  }
  function focusView() {
    if (disposing || !svc || !window.visible)return
    if(addSheet.item){svc.focusWindow(window,addSheet.item);return}
    if(helpLoader.item){svc.focusWindow(window,helpLoader.item);return}
    if(settingsSheet){svc.focusWindow(window,settingsSheet);return}
    if(logView){svc.focusWindow(window,logView);return}
    var target=!svc.online && !svc.startupRequested ? startButton : currentView==="search" && search ? (search.viewModel.mode==="browse" ? search : search.queryTarget) : currentView==="torrents" && torrents ? torrents : currentView==="shows" && shows ? shows : currentView==="rss" && rss ? rss : scope
    svc.focusWindow(window,target)
  }
  function open(payloadJson) {
    var payload={},searchWasOpen=!!root.search
    routeError=""
    if(payloadJson) {
      if(String(payloadJson).length>65536) {routeError="Window request is too large";return false}
      try {payload=JSON.parse(String(payloadJson))}catch(error){routeError="Window request is not valid JSON";return false}
      if(!payload || typeof payload!=="object" || Array.isArray(payload)){routeError="Window request must be an object";return false}
    }
    if(payload.view!==undefined && views.indexOf(payload.view)<0){routeError="Unknown view";return false}
    // Changing views can create Search synchronously while the window is open.
    // Give its initialQuery the new payload before activating that Loader.
    if(typeof payload.query==="string")query=payload.query.slice(0,200)
    if(payload.view!==undefined)currentView=payload.view
    if(typeof payload.filter==="string")filter=payload.filter.slice(0,80)
    if(typeof payload.hash==="string" && /^[0-9a-f]{40}$/.test(payload.hash)){selectedHash=payload.hash;if(torrents)torrents.cursorHash=payload.hash}
    if(svc)svc.closePopups()
    closingFromHost=false
    window.visible=true
    if(svc && !svc.online)svc.startDaemon()
    if(payload.add){
      if(!Source.normalize(payload.add.source)){routeError="Invalid add source";return false}
      openAdd(payload.add.source,payload.add)
    }else if(payload.hash && svc && svc.client && svc.client.caps.indexOf("staging")>=0){
      var hash=payload.hash
      svc.client.request("torrents.get",{hash:hash},{callback:function(result){if(result && result.torrent.staged)svc.addFlow.existing(hash)}})
    }
    publish()
    if(searchWasOpen && payload.query!==undefined && currentView==="search")Qt.callLater(function(){if(root.search)root.search.open(payload.query,true)})
    Qt.callLater(focusView)
    return true
  }
  function close() {
    closingFromHost=true;window.visible=false;publish();closingFromHost=false
  }
  function dismiss() {
    close()
    if(shell && typeof shell.hide==="function")shell.hide("oliver.torrents")
  }
  function textEditing(){var item=window.contentItem.Window.window.activeFocusItem;return !!item && item.cursorPosition!==undefined && item.readOnly!==true}
  function requestStop(){if(!svc || !svc.online)return;if(svc.needsStopConfirm()){stopConfirm.selectedIndex=0;stopConfirm.opened=true;stopConfirm.forceActiveFocus()}else svc.quit()}
  function openSettings(){if(currentView!=="settings")settingsReturnView=currentView;currentView="settings";publish();Qt.callLater(focusView)}
  function selectView(index) {if(views[index]==="settings"){openSettings();return}currentView=views[index];publish();Qt.callLater(focusView);if(currentView==="search")Qt.callLater(function(){if(root.search)root.search.focusQuery()})}
  function cycleView(direction) {selectView((Math.max(0,views.indexOf(currentView))+direction+4)%4)}
  function isWindowActive(){var backing=window.contentItem.Window.window;return !!backing && backing.active}
  function rowPoint(index){return torrents ? torrents.rowPoint(index) : null}
  function grab(path){return scope.grabToImage(function(result){result.saveToFile(path)})}
  function scroll(dy){if(torrents)torrents.scroll(dy)}
  function setPaneWidth(px){return torrents ? torrents.setPaneWidth(px) : 0}
  function resize(width,height){window.implicitWidth=width;window.implicitHeight=height}
  function windowState() {return JSON.stringify(svc ? svc.dashboardState : null)}
  onSvcChanged: if(window && window.visible){publish();focusView()}
  onHelpOpenChanged:{publish();Qt.callLater(focusView)}
  onCurrentViewChanged:publish()
  onFilterChanged:publish()
  onSelectedHashChanged:publish()
  onQueryChanged:publish()
  Connections {target:root.svc;function onRssRevisionChanged(){root.publish()} function onShowsRevisionChanged(){root.publish()} function onSettingsRevisionChanged(){root.publish()} function onClipboardOfferChanged(){root.publish()} function onStatusChanged(){root.publish()} function onAddRevisionChanged(){root.publish()} function onOnlineChanged(){if(!root.svc.online)stopConfirm.opened=false;Qt.callLater(root.focusView)}}
  Component.onDestruction:{disposing=true;if(svc){if(svc.dashboardOwner===root){svc.dashboardVisible=false;svc.dashboardState=null;svc.dashboardOwner=null}svc.cancelWindowFocus(window);svc.hideDetail()}}

  FloatingWindow {
    id:window
    title:"fbittorrent"
    visible:false
    color:Commons.Color.background
    minimumSize:Qt.size(Commons.Style.space(900),Commons.Style.space(560))
    implicitWidth:Commons.Style.space(1600)
    implicitHeight:Commons.Style.space(900)
    onWidthChanged:root.publish()
    onHeightChanged:root.publish()
    onVisibleChanged: {
      if(!visible)root.torrentsVisited=false
      root.publish()
      if(!visible && !root.closingFromHost)root.dismiss()
    }
    FocusScope {
      id:scope
      anchors.fill:parent
      // Include the window background in item captures as well as live rendering.
      Rectangle {anchors.fill:parent;color:Commons.Color.background;z:-1}
      focus:true
      Keys.priority:Keys.AfterItem
      Keys.onPressed:function(event) {
        if(stopConfirm.opened){stopConfirm.handleKey(event);event.accepted=true;return}
        if(helpLoader.active){event.accepted=true;return}
        if(settingsLoader.active)return
        if(addSheet.active){if(addSheet.item && addSheet.item.handleKey(event))event.accepted=true;return}
        if(root.svc && root.svc.clipboardOffer && !(root.currentView==="search" && root.search && root.search.editing) && !(root.currentView==="torrents" && torrents && torrents.editing) && !(root.currentView==="shows" && root.shows && root.shows.dialogOpen) && !(root.currentView==="rss" && root.rss && root.rss.dialogOpen)){
          if(event.text==="y"){root.svc.acceptOffer();event.accepted=true;return}
          if(event.key===Qt.Key_Escape){root.svc.cancelOffer();event.accepted=true;return}
        }
        if(root.currentView==="torrents" && root.svc && root.svc.online && torrents && torrents.handleKey(event)){event.accepted=true;return}
        if(root.currentView==="search" && root.search && root.search.handleKey(event)){event.accepted=true;return}
        if(root.currentView==="shows" && root.shows && root.shows.handleKey(event)){event.accepted=true;return}
        if(root.currentView==="shows" && root.shows && root.shows.editing)return
        if(root.currentView==="rss" && root.rss){if(root.rss.handleKey(event))event.accepted=true;if(event.accepted || root.rss.editing || root.rss.dialogOpen)return}
        if((root.currentView==="torrents" && torrents && torrents.editing) || (root.currentView==="search" && root.search && root.search.editing))return
        if(event.key===Qt.Key_Q && (event.text==="Q" || (event.modifiers & Qt.ControlModifier))){root.requestStop();event.accepted=true;return}
        if(event.text==="r"){root.svc.refresh();event.accepted=true;return}
        if(event.text===","){root.openSettings();event.accepted=true;return}
        if(event.text==="L"){root.selectView(5);event.accepted=true;return}
        if(event.text==="?"){root.helpOpen=true;event.accepted=true;return}
        if(event.text==="z" && !(event.modifiers & (Qt.ControlModifier|Qt.AltModifier))){root.svc.toggleTurtle();event.accepted=true;return}
        if(event.text==="a"){root.svc.addClipboardSheet();event.accepted=true;return}
        if(event.key===Qt.Key_Escape) {
          if(root.query!==""){root.query="";event.accepted=true}
          else if(root.selectedHash!==""){root.selectedHash="";event.accepted=true}
          // At the top level Esc leaves this long-lived window open.
        } else if(event.key===Qt.Key_Q && !(event.modifiers & (Qt.ControlModifier|Qt.AltModifier))) {
          root.dismiss();event.accepted=true
        } else if(event.key>=Qt.Key_1 && event.key<=Qt.Key_4) {
          root.selectView(event.key-Qt.Key_1);event.accepted=true
        }
      }
      Shortcut {sequence:"Y";context:Qt.WindowShortcut;enabled:window.visible && !!root.svc && !!root.svc.clipboardOffer && !addSheet.active && !settingsLoader.active && !helpLoader.active && !root.torrentsEditing && (!root.search || !root.search.editing) && (!root.shows || !root.shows.dialogOpen) && (!root.rss || !root.rss.dialogOpen);onActivated:root.svc.acceptOffer()}
      Shortcut {sequence:"Esc";context:Qt.WindowShortcut;enabled:window.visible && !!root.svc && !!root.svc.clipboardOffer && !addSheet.active && !settingsLoader.active && !helpLoader.active && !root.torrentsEditing && (!root.search || !root.search.editing) && (!root.shows || !root.shows.dialogOpen) && (!root.rss || !root.rss.dialogOpen);onActivated:root.svc.cancelOffer()}
      Shortcut {sequence:"Ctrl+V";context:Qt.WindowShortcut;enabled:window.visible && !stopConfirm.opened && !addSheet.active && !settingsLoader.active && !helpLoader.active && !root.torrentsEditing && (!root.search || !root.search.editing) && (!root.shows || !root.shows.dialogOpen) && (!root.rss || !root.rss.dialogOpen);onActivated:root.svc.addClipboardSheet()}
      Shortcut {sequence:"Ctrl+C";context:Qt.WindowShortcut;enabled:window.visible && !stopConfirm.opened && !addSheet.active && !settingsLoader.active && !helpLoader.active && !root.torrentsEditing && (!root.search || !root.search.editing) && (!root.shows || !root.shows.dialogOpen) && (!root.rss || !root.rss.dialogOpen);onActivated:root.dismiss()}
      Shortcut {sequence:"Shift+Q";context:Qt.WindowShortcut;enabled:window.visible && !!root.svc && root.svc.online && !stopConfirm.opened && !root.textEditing();onActivated:root.requestStop()}
      Shortcut {sequence:"Ctrl+Q";context:Qt.WindowShortcut;enabled:window.visible && !!root.svc && root.svc.online && !stopConfirm.opened;onActivated:root.requestStop()}
      Shortcut {sequence:"Tab";context:Qt.WindowShortcut;enabled:window.visible && !stopConfirm.opened && !addSheet.active && !settingsLoader.active && !helpLoader.active && !root.torrentsDialogOpen && (!root.search || !root.search.dialogOpen) && (!root.shows || !root.shows.dialogOpen) && (!root.rss || !root.rss.dialogOpen);onActivated:root.cycleView(1)}
      Shortcut {sequence:"Shift+Tab";context:Qt.WindowShortcut;enabled:window.visible && !stopConfirm.opened && !addSheet.active && !settingsLoader.active && !helpLoader.active && !root.torrentsDialogOpen && (!root.search || !root.search.dialogOpen) && (!root.shows || !root.shows.dialogOpen) && (!root.rss || !root.rss.dialogOpen);onActivated:root.cycleView(-1)}
      ColumnLayout {
        anchors.fill:parent
        anchors.margins:Commons.Style.space(0)
        spacing:Commons.Style.spacing.panelGap
        RowLayout {
          Layout.fillWidth:true
          Layout.margins:Commons.Style.spacing.panelPadding
          Text {text:"fbittorrent";textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.heading;font.bold:true}
          Item {Layout.fillWidth:true}
          Ui.Button {objectName:"dashboard-turtle";visible:!!root.svc && root.svc.hasCap("limits");text:Glyphs.turtle+" Turtle "+(root.svc && root.svc.turtleOn ? "on" : "off")+" (z)";onClicked:root.svc.toggleTurtle()}
          Ui.Button {text:"Add (a)";onClicked:root.svc.addClipboardSheet()}
          Components.ViewTabs {badges:root.svc ? root.svc.badgeCounts : ({});onChosen:function(index){root.selectView(index)}}
        }
        Components.ClipboardOffer {Layout.fillWidth:true;Layout.leftMargin:Commons.Style.spacing.panelPadding;Layout.rightMargin:Commons.Style.spacing.panelPadding;service:root.svc}
        Components.PopupBanner {Layout.fillWidth:true;Layout.leftMargin:Commons.Style.spacing.panelPadding;Layout.rightMargin:Commons.Style.spacing.panelPadding;text:{if(!root.svc)return "";var revision=root.svc.status;return PopupState.describe(root.svc).banner} urgent:true}
        Ui.PanelSeparator {Layout.fillWidth:true}
        Item {
          Layout.fillWidth:true;Layout.fillHeight:true
          Column {
            anchors.centerIn:parent
            width:Math.min(parent.width,Commons.Style.space(500))
            spacing:Commons.Style.spacing.panelGap
            visible:!root.svc || !root.svc.online
            Text {
              width:parent.width;textFormat:Text.PlainText;horizontalAlignment:Text.AlignHCenter
              objectName:"dashboard-stopped"
              text:root.svc && root.svc.backendState==="protocol_mismatch" ? root.svc.lastError : root.svc && root.svc.reconnecting ? "Reconnecting to fbittorrent…" : root.svc && root.svc.startupRequested ? "Starting fbittorrent…" : "fbittorrent stopped";wrapMode:Text.Wrap
              color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.title
            }
            Ui.Button {id:startButton;anchors.horizontalCenter:parent.horizontalCenter;text:"Start";focusable:true;enabled:!!root.svc && !root.svc.startupRequested && root.svc.backendState!=="protocol_mismatch";onClicked:root.svc.startDaemon()}
          }
          Loader {
            id:torrentsLoader
            asynchronous:true
            visible:root.currentView==="torrents"
            onActiveChanged:if(!active && root.svc)root.svc.cancelWindowFocus(window)
            anchors.fill:parent
            // Keep one table per open window. Changing views must not rebuild
            // the proxy/delegates; visibility releases detail subscriptions.
            active:window.visible && !!root.svc && root.svc.online && (root.currentView==="torrents" || root.torrentsVisited)
            onLoaded:{root.torrentsVisited=true;root.publish();Qt.callLater(root.focusView)}
            sourceComponent:Views.TorrentsView {
              service:root.svc
              windowLogicalWidth:window.width/Commons.Style.effectiveSpacingScale
              cursorHash:root.selectedHash
              onCursorHashChanged:root.selectedHash=cursorHash
              query:root.tableQuery
              onQueryChanged:root.tableQuery=query
              filter:root.filter
              onFilterChanged:root.filter=filter
              onChanged:root.publish()
              onRouted:function(view){root.selectView(root.views.indexOf(view))}
              onDismissRequested:root.dismiss()
            }
          }
          Components.EmptyState {
            anchors.centerIn:parent;width:Math.min(parent.width,Commons.Style.space(440))
            visible:root.currentView==="torrents" && torrentsLoader.status===Loader.Loading
            title:"Loading torrents…";message:""
          }
          Loader {
            id:searchLoader
            anchors.fill:parent
            active:window.visible && !!root.svc && root.svc.online && root.currentView==="search"
            onActiveChanged:if(!active && root.svc)root.svc.cancelWindowFocus(window)
            onLoaded:{root.publish();Qt.callLater(root.focusView)}
            sourceComponent:Views.SearchView {
              service:root.svc;initialQuery:root.query
              onQueryEdited:function(text){root.query=text}
              onChanged:root.publish()
              onDismissRequested:root.dismiss()
            }
          }
          Loader {
            id:logLoader;anchors.fill:parent
            active:window.visible && !!root.svc && root.svc.online && root.currentView==="log"
            onLoaded:{root.publish();Qt.callLater(root.focusView)}
            sourceComponent:Views.LogView {service:root.svc;consumer:"global-log";onChanged:root.publish()}
          }
          Loader {
            id:rssLoader;anchors.fill:parent
            active:window.visible && !!root.svc && root.svc.online && root.currentView==="rss"
            onLoaded:{root.publish();Qt.callLater(root.focusView)}
            sourceComponent:Views.RssView {service:root.svc;onChanged:root.publish();onDismissRequested:root.dismiss();onFocusRequested:root.focusView()}
          }
          Loader {
            id:showsLoader;anchors.fill:parent
            active:window.visible && !!root.svc && root.svc.online && root.currentView==="shows"
            onLoaded:{root.publish();Qt.callLater(root.focusView)}
            sourceComponent:Views.ShowsView {service:root.svc;onChanged:root.publish();onFocusRequested:Qt.callLater(root.focusView)}
          }

        }
        Text {objectName:"dashboard-idle-countdown";Layout.fillWidth:true;Layout.leftMargin:Commons.Style.spacing.panelPadding;text:root.svc ? root.svc.idleLabel : "";visible:text!=="";textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.caption}
        Components.RateStatus {Layout.fillWidth:true;Layout.leftMargin:Commons.Style.spacing.panelPadding;Layout.rightMargin:Commons.Style.spacing.panelPadding;service:root.svc}
        Text {
          visible:root.routeError!=="" || (!!root.svc && root.svc.lastError!=="")
          text:root.routeError || (root.svc ? root.svc.lastError : "")
          textFormat:Text.PlainText;Layout.fillWidth:true;wrapMode:Text.Wrap
          color:Commons.Color.urgent;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.bodySmall
        }
      }
      DropArea {anchors.fill:parent;onEntered:function(drag){drag.accepted=(drag.hasUrls && drag.urls.length===1) || drag.hasText} onDropped:function(drop){if(root.acceptDrop(drop.urls,drop.text))drop.acceptProposedAction();else drop.accepted=false}}
      Loader {id:settingsLoader;anchors.fill:parent;active:window.visible && !!root.svc && root.svc.online && root.currentView==="settings";onLoaded:{root.publish();Qt.callLater(root.focusView)} sourceComponent:Views.SettingsSheet {service:root.svc;onDismissed:root.selectView(root.views.indexOf(root.settingsReturnView))}}
      Loader {id:helpLoader;anchors.fill:parent;active:window.visible && !!root.svc && root.helpOpen;onLoaded:{root.publish();Qt.callLater(root.focusView)} sourceComponent:Views.HelpOverlay {service:root.svc;onDismissed:root.helpOpen=false}}
      Ui.ConfirmDialog {id:stopConfirm;Keys.priority:Keys.BeforeItem;Keys.onPressed:function(event){stopConfirm.handleKey(event);event.accepted=true} anchors.fill:parent;z:1000;selectedIndex:0;confirmText:"Stop";message:root.svc ? root.svc.shutdownPrompt() : "";onOpenedChanged:root.publish();onSelectedIndexChanged:root.publish();onCanceled:{opened=false;Qt.callLater(root.focusView)} onConfirmed:{opened=false;root.svc.quit()}}
      Loader {id:addSheet;anchors.fill:parent;focus:true;onLoaded:root.focusView();onActiveChanged:if(!active && root.svc){root.svc.cancelWindowFocus(window);Qt.callLater(root.focusAfterAdd)} active:window.visible && !!root.svc && !!root.svc.addFlow && root.svc.addRevision>=0 && root.svc.addFlow.snapshot().visible;sourceComponent:Views.AddSheet {service:root.svc;onChanged:root.publish()}}
    }
  }
}
