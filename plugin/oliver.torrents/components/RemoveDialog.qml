import QtQuick
import qs.Ui as Ui
import qs.Commons as Commons
import "../Model.js" as Model

Ui.ConfirmDialog {
  id:root
  property var torrent:null
  property bool deleteFiles:false
  signal removeRequested(string hash,bool files)
  selectedIndex:0
  confirmText:deleteFiles ? "Delete files" : "Remove, keep files"
  message:!torrent ? "" : deleteFiles
    ? "Delete '"+torrent.name+"' and its files? "+Model.size(torrent.size,false)+" will be deleted from "+torrent.save_path+". This cannot be undone."
    : "Remove '"+torrent.name+"'? "+Model.size(torrent.size,false)+" in "+torrent.save_path
  function show(row,files) {torrent={hash:row.hash,name:row.name,size:row.size,save_path:row.save_path};deleteFiles=files;selectedIndex=0;opened=true}
  function key(event) {
    if(!opened)return false
    if(event.text==="y"){confirmed();return true}
    return handleKey(event)
  }
  onCanceled:opened=false
  onConfirmed:{if(torrent)removeRequested(torrent.hash,deleteFiles);opened=false}
}
