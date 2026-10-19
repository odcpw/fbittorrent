import QtQuick
import QtQuick.Layouts
import qs.Ui as Ui
import qs.Commons as Commons
import "../components" as Components
import "../Model.js" as Model
import "../Glyphs.js" as Glyphs
import "../TableLayout.js" as TableLayout

FocusScope {
  id:root
  required property var service
  property real windowLogicalWidth:width/Commons.Style.effectiveSpacingScale
  property string filter:"all"
  property string query:""
  property string cursorHash:""
  property string focusPane:"table"
  property var selection:({})
  property bool detailsOpen:false
  readonly property int tier:TableLayout.tier(windowLogicalWidth)
  readonly property bool dialogOpen:removeDialog.opened || recheckConfirm.opened || (pane && pane.trackerEditing)
  readonly property var pane:docked.item || floating.item
  readonly property bool editing:filterInput.activeFocus || (pane ? pane.editing : false)
  readonly property real scale:Commons.Style.effectiveSpacingScale
  readonly property real dockWidth:Math.max(300,Math.min(service.detailsWidth,480,maxDockWidth))
  readonly property real maxDockWidth:windowLogicalWidth-TableLayout.chrome(windowLogicalWidth)+360-(TableLayout.columns(windowLogicalWidth,0,1).reduce(function(sum,c){return sum+(c.name==="name" ? 0 : c.width)},0))-210
  readonly property bool showingDetail:visible && (tier>=2 || detailsOpen)
  readonly property var detailState:({visible:detailsOpen,tab:service.detailRevision>=0 && service.detailFlow ? service.detailFlow.tab : "overview",
    hash:service.detailRevision>=0 && service.detailFlow ? service.detailFlow.hash : "",width:tier>=2 ? dockWidth : sheetFrame.width/scale,
    mode:tier>=2 ? "docked" : "sheet",files:service.detailRevision>=0 && service.detailFlow ? service.detailFlow.files.map(function(f){return f.index}) : [],poster:pane && pane.posterLoaded ? "loaded" : "none"})
  function setPaneWidth(px){service.detailsWidth=Math.max(300,Math.min(480,maxDockWidth,Number(px) || 360));return dockWidth}
  function scheduleDetail(){if(showingDetail)service.scheduleDetail(cursorHash);else service.hideDetail()}
  onCursorHashChanged:scheduleDetail()
  onShowingDetailChanged:scheduleDetail()
  Component.onCompleted:scheduleDetail()
  Component.onDestruction:service.hideDetail()
  readonly property var current:service.detailRevision>=0 ? service.store.torrents[cursorHash] || null : null
  readonly property var viewState:({errors:sidebar.count("error"),columns:table.columns,sort:{key:service.sortKey,reverse:service.sortReverse},
    toast:service.toast,cursor:cursorHash,selection:Object.keys(selection),filter:filter,query:query,
    focus:focusPane,details:detailState,dialog:removeDialog.opened ? {kind:removeDialog.deleteFiles ? "delete" : "remove",selected:removeDialog.selectedIndex} : recheckConfirm.opened ? {kind:"recheck",selected:recheckConfirm.selectedIndex} : null,
    panes:{sidebar:sideFrame.visible,detailsDocked:detailsFrame.visible,detailsSheet:sheet.visible},
    hover:table.hoverHash,rows:table.count,listWidth:table.width})
  signal routed(string view)
  signal dismissRequested()
  signal changed()
  onViewStateChanged:changed()
  function focusFilter(){filterInput.forceActiveFocus()}
  function focusTable(){filterInput.focus=false;forceActiveFocus()}
  function rowPoint(index){return table.rowPoint(index)}
  function scroll(dy){table.scroll(dy)}
  function choose(hash,modifiers) {
    if(modifiers & Qt.ControlModifier){var next=Object.assign({},selection);if(next[hash])delete next[hash];else next[hash]=true;selection=next}
    else if(modifiers & Qt.ShiftModifier)selection=service.store.selectRange(service.store.visibleKeys(filter,query,service.sortKey,service.sortReverse),cursorHash,hash,selection)
    else selection=({})
    cursorHash=hash;focusPane="table";focusTable()
  }
  function activate(hash){cursorHash=hash;detailsOpen=true;focusPane="details";focusTable()}
  function move(delta,absolute,modifiers) {
    if(focusPane==="sidebar"){sidebar.move(delta);return}
    var previous=table.indexOf(cursorHash),index=absolute===undefined ? Math.max(0,Math.min(table.count-1,previous+delta)) : absolute
    var hash=table.hashAt(index)
    if(hash){choose(hash,modifiers || 0);table.reveal(hash)}
  }
  function back() {
    if(focusPane==="details" && pane && pane.editing)return pane.key({key:Qt.Key_Escape})
    if(removeDialog.opened){removeDialog.canceled();return true}
    if(query){query="";focusTable();return true}
    if(detailsOpen || focusPane==="details"){detailsOpen=false;focusPane="table";return true}
    if(filter!=="all"){filter="all";return true}
    if(Object.keys(selection).length){selection=({});return true}
    return false
  }
  function handleKey(event) {
    if(recheckConfirm.opened){recheckConfirm.handleKey(event);return true}
    if(removeDialog.key(event))return true
    if(focusPane==="details" && pane && pane.editing)return pane.key(event)
    if((event.text==="[" || event.text==="]") && service.detailFlow){service.detailFlow.cycle(event.text==="]" ? 1 : -1);return true}
    if(focusPane==="details" && pane && pane.key(event))return true
    if(event.key===Qt.Key_Escape){back();return true}
    if(editing)return false
    service.clearFailures()
    var text=event.text,control=!!(event.modifiers & Qt.ControlModifier)
    if(event.key===Qt.Key_Q){if(text==="Q" || control)service.quit();else dismissRequested();return true}
    if(event.key===Qt.Key_Down || text==="j"){move(1,undefined,event.modifiers);return true}
    if(event.key===Qt.Key_Up || text==="k"){move(-1,undefined,event.modifiers);return true}
    if(event.key===Qt.Key_PageDown){move(table.pageRows(),undefined,event.modifiers);return true}
    if(event.key===Qt.Key_PageUp){move(-table.pageRows(),undefined,event.modifiers);return true}
    if(event.key===Qt.Key_Home || text==="g"){move(0,0,event.modifiers);return true}
    if(event.key===Qt.Key_End || text==="G"){move(0,table.count-1,event.modifiers);return true}
    if(text==="h" || event.key===Qt.Key_Left){focusPane=focusPane==="details" ? "table" : tier ? "sidebar" : "table";return true}
    if(text==="l" || event.key===Qt.Key_Right){focusPane=focusPane==="sidebar" ? "table" : "details";if(focusPane==="details")detailsOpen=true;return true}
    if(text==="/"){focusFilter();return true}
    if(text==="s"){service.cycleSort();return true}
    if(text==="S"){service.sortReverse=!service.sortReverse;return true}
    if(event.key===Qt.Key_Return || event.key===Qt.Key_Enter){if(current)activate(cursorHash);return true}
    if(text==="o"){if(current)service.openTarget(cursorHash);return true}
    if(text==="p" || event.key===Qt.Key_Space){if(current)service.toggleTorrent(current);return true}
    if(text==="u"){if(current)service.toggleSeed(cursorHash);return true}
    if(text==="d" || text==="x" || event.key===Qt.Key_Delete || text==="D"){if(current)removeDialog.show(current,text==="D");return true}
    if(text==="f"){if(current)service.follow(cursorHash);return true}
    if(text==="i" || text==="w"){if(current)service.titleLink(cursorHash,text==="w");return true}
    if(text==="y"){if(current)service.copyMagnet(cursorHash);return true}
    return false
  }
  Keys.priority:Keys.BeforeItem
  Keys.onPressed:function(event){if(root.handleKey(event))event.accepted=true}
  ColumnLayout {
    anchors.fill:parent
    spacing:Commons.Style.spacing.panelGap
    RowLayout {
      Layout.fillWidth:true
      Layout.leftMargin:Commons.Style.spacing.panelPadding;Layout.rightMargin:Commons.Style.spacing.panelPadding
      spacing:Commons.Style.spacing.panelGap
      Ui.BorderSurface {
        Layout.fillWidth:true;Layout.maximumWidth:Commons.Style.space(400);implicitHeight:Commons.Style.space(32)
        color:Commons.Color.background;borderSpec:Commons.Border.controlSpec(filterInput.activeFocus ? "focus" : "normal",Commons.Color.foreground,Commons.Color.accent)
        Ui.FilterInput {
          id:filterInput
          focus:false
          anchors.fill:parent;anchors.margins:Commons.Style.spacing.sm
          text:root.query;maximumLength:200
          font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body;color:Commons.Color.foreground
          onTextEdited:root.query=text
          Keys.onPressed:function(event){if(event.key===Qt.Key_Escape){root.back();event.accepted=true}else if(event.key===Qt.Key_Down || event.key===Qt.Key_Return){root.focusTable();root.move(1);event.accepted=true}}
          Text {visible:!parent.text && !parent.inputMethodComposing;text:"/ filter…";textFormat:Text.PlainText;color:Model.dim(Commons.Color.foreground,Commons.Color.background);font:parent.font}
        }
      }
      Text {text:"↓ "+Model.rate(root.service.status.dl,true)+"  ↑ "+Model.rate(root.service.status.up,true);textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
      Text {visible:sidebar.vpn;text:Glyphs.vpn+" "+(root.service.store.network.selection || root.service.store.settings.network_interface || "");textFormat:Text.PlainText;color:root.service.store.network.ready===false ? Commons.Color.urgent : Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
      Item {Layout.fillWidth:true}
    }
    Components.TorrentFilters {
      visible:root.tier===0;Layout.fillWidth:true;Layout.leftMargin:Commons.Style.spacing.lg;Layout.rightMargin:Commons.Style.spacing.lg
      service:root.service;selected:root.filter;chips:true
      onChosen:function(key){root.filter=key};onRouted:function(view){root.routed(view)}
    }
    RowLayout {
      Layout.fillWidth:true;Layout.fillHeight:true
      Layout.leftMargin:Commons.Style.space(root.tier>=2 ? 16 : root.tier===1 ? 8 : 0)
      Layout.rightMargin:Layout.leftMargin
      spacing:Commons.Style.space(14)
      Components.PaneFrame {
        id:sideFrame
        visible:root.tier>0;Layout.preferredWidth:Commons.Style.space(200);Layout.fillHeight:true
        Components.TorrentFilters {id:sidebar;anchors.fill:parent;service:root.service;selected:root.filter;focused:root.focusPane==="sidebar";onChosen:function(key){root.filter=key};onRouted:function(view){root.routed(view)}}
      }
      Components.PaneFrame {
        Layout.fillWidth:true;Layout.fillHeight:true
        Column {
          anchors.fill:parent
          spacing:Commons.Style.spacing.xxs
          Rectangle {width:parent.width;height:Commons.Style.space(2);color:Commons.Color.accent;opacity:root.focusPane==="table" ? 1 : 0}
          Components.TorrentTable {
            id:table;width:parent.width;height:parent.height-Commons.Style.space(2)-parent.spacing
            service:root.service;windowLogicalWidth:root.windowLogicalWidth;stateFilter:root.filter;textFilter:root.query
            cursorHash:root.cursorHash;selection:root.selection
            onHovered:function(hash){if(root.focusPane==="table")root.cursorHash=hash}
            onChosen:function(hash,modifiers){root.choose(hash,modifiers)}
            onActivated:function(hash){root.activate(hash)}
            onSortRequested:function(key){root.service.sortBy(key)}
            onGeometryChanged:root.changed()
          }
        }
        Components.EmptyState {
          anchors.centerIn:parent;width:Math.min(parent.width,Commons.Style.space(440))
          visible:table.count===0
          title:!root.service.status.loaded ? "Loading torrents…" : root.service.store.counts.total===0 ? "Nothing here yet." : "No torrents match this filter."
          message:!root.service.status.loaded ? "Waiting for the daemon snapshot." : root.service.store.counts.total===0 ? "Copy a magnet link and press a, or press / to search." : "Press Esc to clear this filter."
        }
      }
      Components.PaneFrame {
        id:detailsFrame
        visible:root.tier>=2;Layout.preferredWidth:Commons.Style.space(root.dockWidth);Layout.fillHeight:true
        Loader {id:docked;anchors.fill:parent;active:root.tier>=2;sourceComponent:detailComponent}
        MouseArea {
          width:Commons.Style.space(10);height:parent.height;anchors.right:parent.left;cursorShape:Qt.SizeHorCursor
          property real pressX:0
          property real startWidth:0
          onPressed:function(mouse){pressX=mapToItem(root,mouse.x,mouse.y).x;startWidth=root.dockWidth}
          onPositionChanged:function(mouse){if(pressed)root.setPaneWidth(startWidth+(pressX-mapToItem(root,mouse.x,mouse.y).x)/root.scale)}
        }
      }
    }
    Components.KeyHint {Layout.fillWidth:true;Layout.leftMargin:Commons.Style.spacing.panelPadding;text:root.focusPane==="sidebar" ? "j/k filter   l table   / filter" : "Enter details   p pause   u seed   d remove   D delete files   o open   s sort   / filter   q close"}
    RowLayout {
      Layout.fillWidth:true;Layout.leftMargin:Commons.Style.spacing.panelPadding;Layout.rightMargin:Commons.Style.spacing.panelPadding
      Text {
        Layout.fillWidth:true
        readonly property string message:root.service.toast ? root.service.toast.text : root.service.lastError || root.service.lastMessageText || root.service.actionStatus
        readonly property real at:root.service.toast ? root.service.toast.until-5000 : root.service.lastMessageAt
        text:message+(message && at ? " · "+Model.when(Math.floor(at/1000)) : "")
        textFormat:Text.PlainText;color:root.service.lastError ? Commons.Color.urgent : Commons.Color.foreground
        font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.caption;elide:Text.ElideRight
        MouseArea {anchors.fill:parent;acceptedButtons:Qt.LeftButton;onClicked:root.routed("log")}
      }
      Ui.Button {visible:!!root.service.lastError;text:"Refresh";onClicked:root.service.refresh()}
    }
  }
  Rectangle {
    id:sheet
    visible:root.detailsOpen && root.tier<2
    anchors.fill:parent;color:Commons.Util.alpha(Commons.Color.background,0.7)
    MouseArea {anchors.fill:parent;onClicked:{root.detailsOpen=false;root.focusPane="table"}}
    Components.PaneFrame {id:sheetFrame;width:Math.min(parent.width-2*Commons.Style.spacing.panelGap,Commons.Style.space(720));height:parent.height-Commons.Style.space(32);anchors.right:parent.right;anchors.margins:Commons.Style.space(16);anchors.verticalCenter:parent.verticalCenter
      MouseArea {anchors.fill:parent;onClicked:{}}
      Loader {id:floating;anchors.fill:parent;active:sheet.visible;sourceComponent:detailComponent}
    }
  }
  Component {id:detailComponent;DetailsPane {service:root.service;focused:root.focusPane==="details";onRecheckRequested:function(hash){recheckConfirm.hash=hash;recheckConfirm.selectedIndex=0;recheckConfirm.opened=true}}}
  Ui.ConfirmDialog {
    id:recheckConfirm
    property string hash:""
    anchors.fill:parent;selectedIndex:0;confirmText:"Recheck";message:"Recheck this torrent's files? File selection is locked until checking finishes."
    onCanceled:opened=false
    onConfirmed:{root.service.recheck(hash);opened=false}
  }
  Components.RemoveDialog {id:removeDialog;anchors.fill:parent;onRemoveRequested:function(hash,files){root.service.remove(hash,files)}}
}
