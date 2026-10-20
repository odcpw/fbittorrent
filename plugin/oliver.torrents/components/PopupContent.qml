import QtQuick
import QtQuick.Controls
import Quickshell
import qs.Commons
import qs.Commons as Commons
import qs.Ui
import "../Model.js" as Model
import "../Glyphs.js" as Glyphs
import "../PopupState.js" as PopupState
PanelKeyCatcher {
  id:root
  required property var owner
  readonly property var svc:owner.svc
  readonly property color foreground:owner.foreground
  readonly property color dim:owner.dim
  readonly property string fontFamily:owner.fontFamily
  readonly property var presentation:{var revision=svc.actionRevision,now=svc.now,status=svc.status;return PopupState.describe(svc)}
  readonly property bool turtleOn:svc.turtleOn
  property bool cursorActive:false
  property bool hints:false
  property int keySerial:0
  property string focusSection:"rows"
  property int rowIndex:0
  property int headerIndex:0
  property int groupIndex:0
  property bool returned:false
  property var pendingRemoval:null
  property bool stopping:false
  property var confirms:({})
  property var expanded:({})
  property var sections:[]
  property var rows:[]
  readonly property int delegateCount:owner.opened && !svc.modelReordering ? rows.length : 0
  property var entries:[]
  ListModel {id:entryModel;dynamicRoles:false}
  function syncEntries(items){
    var same=items.length===entries.length
    if(same)for(var i=0;i<items.length;i++)if((items[i].header ? items[i].group.key : items[i].torrent.hash)!==(entries[i].header ? entries[i].group.key : entries[i].torrent.hash)){same=false;break}
    if(!same){entryModel.clear();for(var j=0;j<items.length;j++)entryModel.append({itemData:items[j]})}
    else for(var k=0;k<items.length;k++)entryModel.setProperty(k,"itemData",items[k])
    entries=items
  }
  implicitHeight:column.implicitHeight
  function registerConfirm(hash,strip){confirms[hash]=strip}
  function unregisterConfirm(hash,strip){if(confirms[hash]===strip)delete confirms[hash]}
  function currentStrip(){return stopping ? stopStrip : pendingRemoval ? confirms[pendingRemoval.hash] : null}
  function rebuild() {
    var groups=[{name:"DOWNLOADING",key:"live",rows:[]},{name:"UPLOADING",key:"seeding",rows:[]},{name:"QUEUED",key:"queued",rows:[]},{name:"PAUSED",key:"paused",rows:[]},{name:"DONE",key:"done",rows:[]},{name:"REVIEWING",key:"reviewing",rows:[]},{name:"OTHER",key:"other",rows:[]}]
    var indexes={};groups.forEach(function(g,i){indexes[g.key]=i})
    if(svc.online)svc.torrents.forEach(function(t){groups[indexes[PopupState.group(t)]].rows.push(t)})
    var all=[],parts=[]
    groups.forEach(function(group){if(!group.rows.length)return;group.start=all.length;group.open=group.key==="live" || group.key==="seeding" || group.key==="other" || group.key==="reviewing" || expanded[group.key]===true;group.visibleRows=group.open ? group.rows.slice(0,10) : [];all=all.concat(group.visibleRows);parts.push(group)})
    var items=[];parts.forEach(function(g){items.push({header:true,group:g});g.visibleRows.forEach(function(t,i){items.push({header:false,torrent:t,index:g.start+i})})});syncEntries(items);sections=parts;rows=all;rowIndex=Math.max(0,Math.min(rowIndex,rows.length-1))
    if(!cursorActive)focusSection=rows.length ? "rows" : "header"
    if(pendingRemoval && !svc.store.torrents[pendingRemoval.hash])cancelRemove()
  }
  function reset(){hints=false;cursorActive=false;pendingRemoval=null;stopping=false;rowIndex=0;headerIndex=0;flick.contentY=0;rebuild();forceActiveFocus()}
  function setHeaderCursor(index){cursorActive=true;focusSection="header";headerIndex=index}
  function setRowCursor(index){cursorActive=true;focusSection="rows";rowIndex=index}
  function selected(){return rows[rowIndex] || null}
  function requestRemoval(t){if(!t || svc.busy)return;pendingRemoval={hash:t.hash,name:t.name};stopping=false;Qt.callLater(function(){var strip=currentStrip();if(strip)strip.reset()})}
  function cancelRemove(){pendingRemoval=null;forceActiveFocus()}
  function confirmRemove(){var target=pendingRemoval;cancelRemove();if(target)svc.remove(target.hash,false)}
  function power(){pendingRemoval=null;if(!svc.online){stopping=false;svc.startDaemon();return}stopping=true;Qt.callLater(function(){stopStrip.reset()})}
  function stop(){stopping=false;svc.quit();forceActiveFocus()}
  function dashboard(view){svc.openDashboard(view || "torrents");owner.close()}
  function toggleGroup(group){var next=Object.assign({},expanded);next[group.key]=!group.open;expanded=next;rebuild()}
  function header(){if(headerIndex===0)svc.addFromClipboard();else if(headerIndex===1)dashboard();else if(headerIndex===2)svc.toggleTurtle();else power()}
  function move(dx,dy) {
    keySerial++;hints=true
    var strip=currentStrip();if(strip){strip.move(dx || dy);return}
    if(!cursorActive){cursorActive=true;return}
    if(focusSection==="header"){if(dx)headerIndex=Math.max(0,Math.min(3,headerIndex+dx));if(dy>0){focusSection=rows.length ? "rows" : "groups";rowIndex=0;groupIndex=0}return}
    if(focusSection==="groups"){if(dy<0 && groupIndex===0){focusSection=rows.length ? "rows" : "header";rowIndex=Math.max(0,rows.length-1)}else groupIndex=Math.max(0,Math.min(sections.length-1,groupIndex+dy));return}
    if(dy>0 && rowIndex===rows.length-1 && sections.length){focusSection="groups";groupIndex=0;return}
    if(dy<0 && rowIndex===0){focusSection="header";return}
    rowIndex=Math.max(0,Math.min(rows.length-1,rowIndex+dy))
    Qt.callLater(function(){var item=null;for(var i=0;i<rowRepeater.count;i++){var entry=rowRepeater.itemAt(i);if(entry && !entry.itemData.header && entry.itemData.index===rowIndex){item=entry;break}}if(!item)return;var y=item.mapToItem(flick.contentItem,0,0).y;if(y<flick.contentY)flick.contentY=y;else if(y+item.height>flick.contentY+flick.height)flick.contentY=y+item.height-flick.height})
  }
  onMoveRequested:function(dx,dy){root.move(dx,dy)}
  onReturnRequested:{returned=true;Qt.callLater(function(){root.returned=false})}
  onActivateRequested:{
    keySerial++;hints=true;var enter=returned;returned=false
    if(svc.clipboardOffer){svc.acceptOffer();return}
    var strip=currentStrip();if(strip){strip.activate();return}
    if(!svc.online){if(enter)svc.startDaemon();return}
    if(focusSection==="header"){header();return}
    if(focusSection==="groups"){var g=sections[groupIndex];if(g){toggleGroup(g);if(rows.length){focusSection="rows";rowIndex=Math.min(g.start,rows.length-1)}}return}
    cursorActive=true;if(enter)svc.openTorrent(selected());else svc.toggleTorrent(selected())
  }
  onCloseRequested:{keySerial++;if(svc.clipboardOffer)svc.cancelOffer();else if(stopping){stopping=false;forceActiveFocus()}else if(pendingRemoval)cancelRemove();else owner.close()}
  onDeleteRequested:{keySerial++;hints=true;if(!currentStrip())requestRemoval(selected())}
  onTabRequested:function(direction){keySerial++;hints=true;var strip=currentStrip();if(strip)strip.move(direction);else owner.switchPanel(direction)}
  onTextKey:function(t){
    keySerial++;hints=true
    var strip=currentStrip();if(strip){if(t==="y")strip.confirmed();return}
    if(svc.clipboardOffer){if(t==="y")svc.acceptOffer();return}
    if(t==="q")owner.close()
    else if(t==="Q")power()
    else if(t==="a")svc.addFromClipboard()
    else if(t==="w"){if(focusSection==="groups" && sections[groupIndex])svc.openDashboardFilter(PopupState.dashboardFilter(sections[groupIndex].key));else svc.openDashboard();owner.close()}
    else if(t==="/")dashboard("search")
    else if(t==="t")owner.openTui()
    else if(t==="z")svc.toggleTurtle()
    else if(t==="p")svc.toggleTorrent(selected())
    else if(t==="o")svc.openTorrent(selected())
    else if(t==="u"){svc.openTorrentDetails(selected());owner.close()}
    else if(t==="d" || t==="x")requestRemoval(selected())
  }
  function cursor(){var strip=currentStrip();return {keySerial:keySerial,qtFocus:activeFocus,section:focusSection,row:rowIndex,header:headerIndex,active:cursorActive,rows:rows.length,hashes:rows.map(function(t){return t.hash}),confirming:!!pendingRemoval,stopping:stopping,confirmFocused:strip && strip.cancelFocused ? "cancel" : "other",selected:strip ? strip.selected : null,group:sections[groupIndex] ? sections[groupIndex].key : "",pendingHash:pendingRemoval ? pendingRemoval.hash : "",hints:hints}}
  Connections {target:svc;function onTorrentsChanged(){root.rebuild()} function onOnlineChanged(){if(!svc.online){root.stopping=false;root.pendingRemoval=null}root.rebuild()} function onClipboardOfferChanged(){if(!svc.clipboardOffer)root.forceActiveFocus()}}
  Connections {target:owner;function onOpenedChanged(){if(owner.opened)root.reset()}}
  Component.onCompleted:reset()
  Flickable {
    id:flick;anchors.fill:parent;contentWidth:width;contentHeight:column.implicitHeight;clip:true;boundsBehavior:Flickable.StopAtBounds;interactive:contentHeight>height
    ScrollBar.vertical:ScrollBar {policy:ScrollBar.AsNeeded}
    Column {
      id:column;width:flick.width;spacing:Style.spacing.xxl
      PanelHero {
        width:parent.width;title:"Torrents";meta:svc.online && svc.status.total===0 && svc.store.counts.reviewing>0 ? "Reviewing…" : Model.heroMeta(svc.status);detail:(svc.online && svc.status.active ? svc.status.active+" DOWNLOADING" : "")+(root.turtleOn ? " · Turtle on" : "");foreground:root.foreground;fontFamily:root.fontFamily
        iconComponent:Component {Text {text:Glyphs.download;color:root.foreground;font.family:Style.font.family;font.pixelSize:Style.font.display}}
        trailingControl:Component {Row {
          spacing:Style.spacing.xxs
          HeaderAction {host:root;index:0;iconText:Glyphs.add;tooltipText:"Add from clipboard (a)";onClicked:svc.addFromClipboard()}
          HeaderAction {host:root;index:1;iconText:"▢";tooltipText:"Open window (w)";onClicked:root.dashboard()}
          HeaderAction {host:root;index:2;objectName:"popup-turtle";iconText:Glyphs.turtle;tooltipText:"Turtle limits "+(root.turtleOn ? "on" : "off")+" (z)";enabled:svc.online && svc.client && svc.client.caps.indexOf("limits")>=0;onClicked:svc.toggleTurtle()}
          HeaderAction {host:root;index:3;objectName:"popup-power";iconText:Glyphs.power;tooltipText:svc.online ? "Stop fbittorrent (Q)" : "Start fbittorrent";enabled:!svc.startupRequested;onClicked:root.power()}
        }}
      }
      Text {visible:text!=="";text:root.presentation.idle;textFormat:Text.PlainText;color:root.dim;font.family:Style.font.family;font.pixelSize:Style.font.caption}
      ConfirmStrip {id:stopStrip;width:parent.width;visible:root.stopping;message:svc.shutdownPrompt();confirmText:"Stop";onCanceled:{root.stopping=false;root.forceActiveFocus()} onConfirmed:root.stop()}
      ClipboardOffer {width:parent.width;service:svc}
      PanelSeparator {foreground:root.foreground}
      PopupBanner {width:parent.width;text:root.presentation.banner;urgent:root.presentation.urgent;showLog:svc.backendState==="error";onLogRequested:root.dashboard("log")}
      Text {width:parent.width;visible:svc.actionStatus!=="" || svc.lastError!=="";text:svc.actionStatus || svc.lastError;textFormat:Text.PlainText;wrapMode:Text.Wrap;color:root.dim;font.family:Style.font.family;font.pixelSize:Style.font.bodySmall}
      Row {visible:svc.resultUnknown;spacing:Style.spacing.md
        Text {text:"Result unknown; the list will tell";textFormat:Text.PlainText;color:root.dim;font.family:Style.font.family;font.pixelSize:Style.font.caption}
        PanelActionButton {iconText:Glyphs.refresh;tooltipText:"Refresh";onClicked:svc.refresh()}
      }
      Text {width:parent.width;visible:root.presentation.message!=="";text:root.presentation.message;textFormat:Text.PlainText;wrapMode:Text.Wrap;color:root.dim;font.family:Style.font.family;font.pixelSize:Style.font.body;topPadding:!root.rows.length ? Style.spacing.xxl : 0;bottomPadding:topPadding}
      Repeater {
        id:rowRepeater
        model:owner.opened && !svc.modelReordering ? entryModel : null
        Loader {
          id:entry
          required property var itemData
          width:column.width
          sourceComponent:itemData.header ? groupHeader : torrentRow
          Component {
            id:groupHeader
            PanelSectionHeader {
              text:entry.itemData.group.name+" "+entry.itemData.group.rows.length+(entry.itemData.group.open ? "" : " ›")
              foreground:root.foreground;color:root.dim
              font.underline:root.cursorActive && root.focusSection==="groups" && root.sections[root.groupIndex] && root.sections[root.groupIndex].key===entry.itemData.group.key
              MouseArea {anchors.fill:parent;onClicked:root.toggleGroup(entry.itemData.group)}
            }
          }
          Component {id:torrentRow;PopupRow {host:root;torrent:entry.itemData.torrent;rowIdx:entry.itemData.index}}
        }
      }
      RateStatus {width:parent.width;service:svc}
      PanelSeparator {visible:root.hints;foreground:root.foreground}
      PopupKeyHints {visible:root.hints}
    }
  }
}
