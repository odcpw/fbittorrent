import QtQuick
import QtQuick.Controls
import qs.Commons as Commons
import qs.Ui as Ui
import "../DetailText.js" as DetailText
import "../Model.js" as Model
Flickable {
 id:root
 required property var service
 property var rows:[]
 property string cursor:""
 property var marked:({})
 property bool loading:false
 property string message:""
 readonly property int columns:Math.max(3,Math.min(6,Math.floor(width/(Commons.Style.space(120)+Commons.Style.spacing.lg*2))))
 readonly property real pitch:Commons.Style.space(120)+Commons.Style.spacing.lg*2
 readonly property real rowPitch:Commons.Style.space(266)
 property int imageRevision:0
 signal chosen(string key)
 signal toggled(string key)
 signal ranged(string key)
 signal activated()
 clip:true;contentWidth:width;contentHeight:tiles.implicitHeight;boundsBehavior:Flickable.StopAtBounds
 ScrollBar.vertical:ScrollBar {}
 function reveal(){var index=rows.findIndex(function(row){return row.key===cursor});if(index<0)return;var y=Math.floor(index/columns)*rowPitch;if(y<contentY)contentY=y;else if(y+rowPitch>contentY+height)contentY=y+rowPitch-height}
 function state(){var revision=imageRevision,ready=0,images=0;for(var i=0;i<cells.count;i++){var item=cells.itemAt(i);if(!item)continue;if(item.hasImage)images++;if(item.posterReady)ready++}return {columns:columns,cells:cells.count,images:images,ready:ready,cursor:cursor}}
 onCursorChanged:reveal()
 onColumnsChanged:reveal()
 EmptyState {anchors.centerIn:parent;width:Math.min(parent.width,Commons.Style.space(420));visible:!root.rows.length;title:root.loading ? "Loading browse list…" : "No browse results";message:root.message}
 Grid {
  id:tiles;x:Math.max(0,(root.width-root.columns*root.pitch)/2);columns:root.columns
  Repeater {
   id:cells;model:root.rows.slice(0,60)
   onItemAdded:root.imageRevision++
   onItemRemoved:root.imageRevision++
   Ui.BorderSurface {
    id:cell
    required property var modelData
    readonly property var card:root.service.browsePosterRevision>=0 && root.service.browsePosters ? root.service.browsePosters.card(modelData.imdb_id) : null
    readonly property bool hasImage:!!(card && card.poster_path)
    readonly property bool posterReady:poster.status===Image.Ready
    width:root.pitch;height:root.rowPitch
    color:modelData.key===root.cursor ? Commons.Style.selectedFill : root.marked[modelData.key] ? Commons.Util.alpha(Commons.Color.accent,0.12) : Commons.Color.background
    borderSpec:Commons.Border.none()
    Column {
     x:Commons.Style.spacing.lg;y:Commons.Style.spacing.sm;width:Commons.Style.space(120);spacing:Commons.Style.spacing.xs
     Item {width:parent.width;height:Commons.Style.space(180)
      Image {id:poster;anchors.fill:parent;source:DetailText.localUrl(cell.card ? cell.card.poster_path : "");asynchronous:true;sourceSize:Qt.size(width,height);fillMode:Image.PreserveAspectFit;onStatusChanged:root.imageRevision++}
      Text {anchors.centerIn:parent;visible:!cell.hasImage;text:"No poster";textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.caption}
     }
     Text {width:parent.width;text:(cell.modelData.key===root.cursor ? "▸ " : "")+(root.marked[cell.modelData.key] ? "✓ " : "")+(cell.modelData.added ? "+ " : cell.modelData.adding ? "… " : "")+(cell.card ? cell.card.title : cell.modelData.name);textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body;wrapMode:Text.Wrap;maximumLineCount:2;elide:Text.ElideRight}
     Text {width:parent.width;text:(cell.card && cell.card.year ? String(cell.card.year)+" " : "")+((cell.modelData.rating!==undefined ? cell.modelData.rating : cell.card ? cell.card.rating : undefined)!==undefined ? "★ "+(cell.modelData.rating!==undefined ? cell.modelData.rating : cell.card.rating) : "");textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.caption;elide:Text.ElideRight}
     Text {visible:cell.modelData.seeds!==undefined;width:parent.width;text:Model.count(cell.modelData.seeds)+" seeds";textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.caption}
    }
    MouseArea {anchors.fill:parent;onClicked:function(mouse){if(mouse.modifiers & Qt.ControlModifier)root.toggled(cell.modelData.key);else if(mouse.modifiers & Qt.ShiftModifier)root.ranged(cell.modelData.key);else root.chosen(cell.modelData.key)} onDoubleClicked:root.activated()}
   }
  }
 }
}
