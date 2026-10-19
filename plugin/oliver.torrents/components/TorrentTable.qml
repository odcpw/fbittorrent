import QtQuick
import QtQuick.Controls
import QtQml.Models
import qs.Commons as Commons
import "../Model.js" as Model
import "../TableLayout.js" as TableLayout

Column {
  id:root
  required property var service
  property string stateFilter:"all"
  property string textFilter:""
  property string cursorHash:""
  property string hoverHash:""
  readonly property bool reordering:service.modelReordering===true
  property real savedScroll:0
  property var selection:({})
  property real windowLogicalWidth:width/Commons.Style.effectiveSpacingScale
  property var columns:TableLayout.columns(windowLogicalWidth,width,Commons.Style.effectiveSpacingScale)
  readonly property int count:list.count
  signal hovered(string hash)
  signal chosen(string hash,int modifiers)
  signal activated(string hash)
  signal sortRequested(string key)
  signal geometryChanged()
  spacing:Commons.Style.spacing.xxs
  function hashAt(index) {
    if(index<0 || index>=list.count)return ""
    var source=proxy.mapToSource(proxy.index(index,0))
    return service.torrentModel.get(source.row).hash
  }
  function indexOf(hash) {
    if(reordering)return -1
    var source=service.store.modelIndex(hash)
    return source<0 ? -1 : proxy.mapFromSource(service.torrentModel.index(source,0)).row
  }
  function reveal(hash) {var index=indexOf(hash);if(index>=0)list.positionViewAtIndex(index,ListView.Contain)}
  function scroll(dy) {list.contentY=Math.max(0,Math.min(Math.max(0,list.contentHeight-list.height),list.contentY+dy))}
  function progress(hash){var item=list.itemAtIndex(indexOf(hash));return item ? item.progress() : 0}
  function stateText(hash){var item=list.itemAtIndex(indexOf(hash));return item ? item.stateText() : ""}
  function rowPoint(index) {
    var item=list.itemAtIndex(index)
    if(!item)return null
    var point=item.mapToItem(null,Math.min(item.width-1,Commons.Style.space(200)),item.height/2)
    return {x:point.x,y:point.y}
  }
  function pageRows() {return Math.max(1,Math.floor(list.height/Commons.Style.spacing.popupRowHeight))}
  onStateFilterChanged:service.store.setFilter(stateFilter,textFilter)
  onTextFilterChanged:service.store.setFilter(stateFilter,textFilter)
  onColumnsChanged:geometryChanged()
  Component.onCompleted:service.store.setFilter(stateFilter,textFilter)
  ListModel {id:emptyRows;dynamicRoles:true}
  // Qt 6.12 resets a proxy on every source move. Detach once around a sort,
  // then restore the viewport; ordinary role patches never change the source.
  Connections {
    target:root.service
    ignoreUnknownSignals:true
    function onStoreChanged(){root.service.store.setFilter(root.stateFilter,root.textFilter)}
    function onRowOrderChanging(){root.savedScroll=list.contentY}
    function onRowOrderChanged(){Qt.callLater(function(){list.contentY=Math.max(0,Math.min(root.savedScroll,Math.max(0,list.contentHeight-list.height)))})}
  }
  SortFilterProxyModel {
    id:proxy
    model:root.reordering ? emptyRows : root.service.torrentModel || emptyRows
    filters:ValueFilter {roleName:"uiVisible";value:true}
  }
  Row {
    id:header
    width:parent.width;height:Commons.Style.space(30)
    Repeater {
      model:root.columns
      Item {
        required property var modelData
        width:modelData.width;height:header.height;visible:!modelData.hidden
        Text {
          anchors.fill:parent
          anchors.leftMargin:Commons.Style.spacing.controlPaddingX
          anchors.rightMargin:Commons.Style.spacing.controlPaddingX
          textFormat:Text.PlainText
          text:modelData.label+(TableLayout.sortKey(modelData.name)===root.service.sortKey ? (TableLayout.descending(root.service.sortKey)!==root.service.sortReverse ? " ↓" : " ↑") : "")
          color:TableLayout.sortKey(modelData.name)===root.service.sortKey ? Commons.Color.foreground : Model.dim(Commons.Color.foreground,Commons.Color.background)
          font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.bodySmall;font.letterSpacing:Commons.Style.font.bodySmall*0.08
          verticalAlignment:Text.AlignVCenter
          horizontalAlignment:["name","state","glyph"].indexOf(modelData.name)>=0 ? Text.AlignLeft : Text.AlignRight
        }
        MouseArea {anchors.fill:parent;acceptedButtons:Qt.LeftButton;onClicked:{var key=TableLayout.sortKey(modelData.name);if(key)root.sortRequested(key)}}
      }
    }
  }
  ListView {
    id:list
    width:parent.width;height:Math.max(0,root.height-header.height-root.spacing)
    clip:true;reuseItems:true;currentIndex:-1
    model:proxy
    boundsBehavior:Flickable.StopAtBounds
    ScrollBar.vertical:ScrollBar {policy:ScrollBar.AsNeeded}
    delegate:TorrentTableRow {
      required property var model
      // A pooled delegate can remain attached to the proxy's content item.
      // Hide it immediately, before its old role bindings lose their hash.
      property bool pooled:false
      ListView.onPooled:pooled=true
      ListView.onReused:pooled=false
      visible:!pooled && !!model.hash
      width:list.width;host:root;torrent:model
      selected:!!root.selection[model.hash]
    }
  }
}
