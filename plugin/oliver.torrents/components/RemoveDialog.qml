import QtQuick
import qs.Ui as Ui
import qs.Commons as Commons
import "../Model.js" as Model

Ui.ConfirmDialog {
  id:root
  property var captured:[]
  readonly property var torrent:captured.length ? captured[0] : null
  property bool deleteFiles:false
  signal removeRequested(var hashes,bool files)
  selectedIndex:0
  confirmText:deleteFiles ? "Delete files" : "Remove, keep files"
  readonly property real bytes:captured.reduce(function(sum,row){return sum+Number(row.size || 0)},0)
  readonly property string subject:captured.length===1 ? "'"+torrent.name+"'" : captured.length+" selected torrents"
  message:!torrent ? "" : deleteFiles
    ? "Delete "+subject+" and its files? "+Model.size(bytes,false)+" will be deleted. This cannot be undone."
    : "Remove "+subject+"? Files stay in their current folders. Undo is available for 10 seconds."
  function show(rows,files) {captured=rows.map(function(row){return {hash:row.hash,name:row.name,size:row.size,save_path:row.save_path}});deleteFiles=files;selectedIndex=0;opened=captured.length>0}
  function key(event) {
    if(!opened)return false
    if(event.text==="y"){confirmed();return true}
    return handleKey(event)
  }
  onCanceled:opened=false
  onConfirmed:{if(captured.length)removeRequested(captured.map(function(row){return row.hash}),deleteFiles);opened=false}
}
