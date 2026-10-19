import QtQuick
import QtQuick.Window
import Quickshell
import "Plugin/components"
import "Plugin/Store.js" as Store
ShellRoot {
  id:root
  ListModel {id:rows;dynamicRoles:true}
  QtObject {
    id:service
    property var store:Store.create()
    property alias torrentModel:rows
    property string sortKey:"added"
    property bool sortReverse:false
    function pendingGlyph(hash){return ""}
    function rowFailed(hash){return false}
  }
  Window {visible:true;width:1600;height:600;TorrentTable {id:table;anchors.fill:parent;service:service;windowLogicalWidth:1600}}
  property int phase:0
  property int attempts:0
  readonly property string hash:"a".repeat(40)
  Timer {interval:20;repeat:true;running:true;onTriggered:{
    if(table.count!==1)return
    if(root.phase===0){
      if(table.stateText(root.hash)!=="Downloading")return
      root.phase=1
      service.store.apply({id:2,ok:true,result:{torrents:[{hash:root.hash,name:"New video",state:"downloading",added_at:1,progress:0.25,playable:{ready:true,stall_free:true,acked:false}}]}},{op:"torrents.list"})
      service.store.syncModel(rows,"added",false)
      return
    }
    var actual=table.stateText(root.hash)
    if((actual!=="Ready to watch" || table.progress(root.hash)!==0.25) && ++root.attempts<50)return
    stop()
    console.log("FBTQS-STEP "+JSON.stringify({case:"late_readiness",action:"receive readiness after the existing native row was created without playback data",expect:"the rendered state text becomes Ready to watch without rebuilding the list",observed:JSON.stringify({text:actual,progress:table.progress(root.hash),model:rows.get(0)}),ok:actual==="Ready to watch" && table.progress(root.hash)===0.25,ms:0}))
    console.log("FBTQS-PASS late_readiness")
    Qt.callLater(function(){Qt.quit()})
  }}
  Component.onCompleted:{service.store.apply({id:1,ok:true,result:{torrents:[{hash:hash,name:"New video",state:"downloading",added_at:1}]}},{op:"torrents.list"});service.store.syncModel(rows,"added",false)}
}
