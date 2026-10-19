import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell
import qs.Commons as Commons
import qs.Ui as Ui
import "Model.js" as Model
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
  property string query:""
  property string filter:"all"
  property string selectedHash:""
  property string tableQuery:""
  property bool closingFromHost:false
  property string routeError:""
  readonly property var torrents:torrentsLoader.item
  readonly property var views:["torrents","search","shows","rss","settings","log"]
  readonly property var labels:["Torrents","Search","Shows","RSS","Settings","Log"]
  function selectTorrent(hash){currentView="torrents";selectedHash=hash;if(torrents)torrents.cursorHash=hash}
  function openAdd(source,options){var accepted=svc ? svc.beginAdd(source,options) : false;Qt.callLater(function(){if(addSheet.item)addSheet.item.focusSheet()});return accepted}
  function acceptDrop(urls,text){var parsed=Source.drop(urls,text);if(!parsed)return false;return openAdd(parsed.source)}
  function publish() {
    if (!svc || !window || !searchInput)return
    if(!window.visible && svc.dashboardOwner!==root)return
    svc.dashboardOwner=window.visible ? root : null
    svc.dashboardVisible=window.visible
    svc.dashboardState=window.visible ? {window:{visible:true,width:window.width,height:window.height,view:currentView},
      cursor:searchInput.activeFocus ? "query" : "view",selection:selectedHash ? [selectedHash] : [],filter:filter,query:query} : null
    if(window.visible && currentView==="torrents" && torrents)svc.dashboardState=Object.assign({},svc.dashboardState,torrents.viewState)
    if(window.visible && svc.addFlow){svc.dashboardState.add=svc.addFlow.snapshot();if(svc.dashboardState.add.visible){svc.dashboardState.cursor="add"}}

  }
  function focusView() {
    if (!svc || !window.visible)return
    if(addSheet.item){svc.focusWindow(window,addSheet.item);return}
    var target=!svc.online && !svc.startupRequested ? startButton : currentView==="search" ? searchInput : currentView==="torrents" && torrents ? torrents : scope
    svc.focusWindow(window,target)
  }
  function open(payloadJson) {
    var payload={}
    routeError=""
    if(payloadJson) {
      if(String(payloadJson).length>65536) {routeError="Window request is too large";return false}
      try {payload=JSON.parse(String(payloadJson))}catch(error){routeError="Window request is not valid JSON";return false}
      if(!payload || typeof payload!=="object" || Array.isArray(payload)){routeError="Window request must be an object";return false}
    }
    if(payload.view!==undefined) {
      if(views.indexOf(payload.view)<0){routeError="Unknown view";return false}
      currentView=payload.view
    }
    if(typeof payload.query==="string")query=payload.query.slice(0,200)
    if(typeof payload.filter==="string")filter=payload.filter.slice(0,80)
    if(typeof payload.hash==="string" && /^[0-9a-f]{40}$/.test(payload.hash)){selectedHash=payload.hash;if(torrents)torrents.cursorHash=payload.hash}
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
  function selectView(index) {currentView=views[index];publish();Qt.callLater(focusView)}
  function cycleView(direction) {selectView((Math.max(0,views.indexOf(currentView))+direction+4)%4)}
  function isWindowActive(){return window._backingWindow && window._backingWindow.active}
  function rowPoint(index){return torrents ? torrents.rowPoint(index) : null}
  function grab(path){return scope.grabToImage(function(result){result.saveToFile(path)})}
  function scroll(dy){if(torrents)torrents.scroll(dy)}
  function resize(width,height){window.implicitWidth=width;window.implicitHeight=height}
  function windowState() {return JSON.stringify(svc ? svc.dashboardState : null)}
  onSvcChanged: if(window && window.visible){publish();focusView()}
  onCurrentViewChanged:publish()
  onFilterChanged:publish()
  onSelectedHashChanged:publish()
  onQueryChanged:publish()
  Connections {target:root.svc;function onAddRevisionChanged(){root.publish()} function onOnlineChanged(){if(root.svc.online)Qt.callLater(root.focusView)}}
  Component.onDestruction:if(svc){if(svc.dashboardOwner===root){svc.dashboardVisible=false;svc.dashboardState=null;svc.dashboardOwner=null}svc.cancelWindowFocus(window)}

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
      root.publish()
      if(!visible && !root.closingFromHost)root.dismiss()
    }
    FocusScope {
      id:scope
      anchors.fill:parent
      focus:true
      Keys.priority:Keys.AfterItem
      Keys.onPressed:function(event) {
        if(addSheet.active){if(addSheet.item && addSheet.item.handleKey(event))event.accepted=true;return}
        if(event.text==="a"){root.svc.addClipboardSheet();event.accepted=true;return}
        if(root.currentView==="torrents" && root.svc && root.svc.online && torrents && torrents.handleKey(event)){event.accepted=true;return}
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
      Shortcut {sequence:"Ctrl+V";context:Qt.WindowShortcut;enabled:window.visible && !addSheet.active;onActivated:root.svc.addClipboardSheet()}
      Shortcut {sequence:"Ctrl+Q";context:Qt.WindowShortcut;enabled:window.visible && !!root.svc && root.svc.online;onActivated:root.svc.quit()}
      Shortcut {sequence:"Tab";context:Qt.WindowShortcut;enabled:window.visible && !addSheet.active && (!torrents || !torrents.dialogOpen);onActivated:root.cycleView(1)}
      Shortcut {sequence:"Shift+Tab";context:Qt.WindowShortcut;enabled:window.visible && !addSheet.active && (!torrents || !torrents.dialogOpen);onActivated:root.cycleView(-1)}
      ColumnLayout {
        anchors.fill:parent
        anchors.margins:Commons.Style.space(0)
        spacing:Commons.Style.spacing.panelGap
        RowLayout {
          Layout.fillWidth:true
          Layout.margins:Commons.Style.spacing.panelPadding
          Text {text:"fbittorrent";textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.heading;font.bold:true}
          Item {Layout.fillWidth:true}
          Ui.Button {text:"Add (a)";onClicked:root.svc.addClipboardSheet()}
          Components.ViewTabs {badges:root.svc ? root.svc.badgeCounts : ({});onChosen:function(index){root.selectView(index)}}
        }
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
              text:root.svc && root.svc.startupRequested ? "Starting fbittorrent…" : "fbittorrent is not running"
              color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.title
            }
            Ui.Button {id:startButton;anchors.horizontalCenter:parent.horizontalCenter;text:"Start";focusable:true;enabled:!!root.svc && !root.svc.startupRequested;onClicked:root.svc.startDaemon()}
          }
          Loader {
            id:torrentsLoader
            anchors.fill:parent
            active:window.visible && !!root.svc && root.svc.online && root.currentView==="torrents"
            onLoaded:{root.publish();Qt.callLater(root.focusView)}
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
          ColumnLayout {
            anchors.fill:parent
            visible:!!root.svc && root.svc.online && root.currentView!=="torrents"
            spacing:Commons.Style.spacing.panelGap
            Text {text:root.labels[root.views.indexOf(root.currentView)];textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.title;font.bold:true}
            Ui.TextField {
              id:searchInput
              visible:root.currentView==="search"
              Layout.fillWidth:true
              placeholderText:"Search titles"
              maximumLength:200
              text:root.query
              onTextEdited:root.query=text
              onActiveFocusChanged:root.publish()
            }
            Item {Layout.fillWidth:true;Layout.fillHeight:true}
          }
        }
        Text {
          visible:root.routeError!=="" || (!!root.svc && root.svc.lastError!=="")
          text:root.routeError || (root.svc ? root.svc.lastError : "")
          textFormat:Text.PlainText;Layout.fillWidth:true;wrapMode:Text.Wrap
          color:Commons.Color.urgent;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.bodySmall
        }
      }
      DropArea {anchors.fill:parent;onEntered:function(drag){drag.accepted=(drag.hasUrls && drag.urls.length===1) || drag.hasText} onDropped:function(drop){if(root.acceptDrop(drop.urls,drop.text))drop.acceptProposedAction();else drop.accepted=false}}
      Loader {id:addSheet;anchors.fill:parent;focus:true;onLoaded:root.focusView();onActiveChanged:if(!active && root.svc){root.svc.cancelWindowFocus(window);Qt.callLater(root.focusView)} active:window.visible && !!root.svc && !!root.svc.addFlow && root.svc.addRevision>=0 && root.svc.addFlow.snapshot().visible;sourceComponent:Views.AddSheet {service:root.svc;onChanged:root.publish()}}
    }
  }
}
