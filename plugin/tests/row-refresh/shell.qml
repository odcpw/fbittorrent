import QtQuick
import QtQuick.Window
import Quickshell
import "Plugin/components"
import "Plugin/Store.js" as Store

ShellRoot {
  id:root
  property int phase:0
  property var delegateBefore:null
  readonly property string hash:"a".repeat(40)
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
  Window {visible:true;width:1600;height:400
    TorrentTable {id:table;anchors.fill:parent;service:service;windowLogicalWidth:1600}
  }
  function check(name,expect,actual,ok){console.log("FBTQS-STEP "+JSON.stringify({case:name,action:"patch the production store/model without sorting",expect:expect,observed:JSON.stringify(actual),ok:!!ok,ms:0}))}
  function texts(item){var out=[];if(typeof item.text==="string")out.push(item.text);for(var i=0;i<item.children.length;i++)out=out.concat(texts(item.children[i]));return out}
  function row(item){if(item.hasCursor!==undefined && item.torrent!==undefined)return item;for(var i=0;i<item.children.length;i++){var found=row(item.children[i]);if(found)return found}return null}
  function list(value){service.store.apply({id:1,ok:true,result:{torrents:[value]}},{op:"torrents.list"});service.store.syncModel(rows,"added",false)}
  Component.onCompleted:{
    list({hash:hash,name:"Before rename",state:"paused",progress:0.5,size:4096,added_at:1,seed:false})
  }
  Timer {interval:50;repeat:true;running:true
    onTriggered:{
      var labels=root.texts(table)
      if(root.phase===0){
        root.delegateBefore=root.row(table)
        root.check("rendered_roles","initial paused row is actually rendered",labels,labels.indexOf("Before rename")>=0 && labels.indexOf("Paused")>=0 && !!root.delegateBefore)
        root.list({hash:root.hash,name:"After rename",state:"done",progress:1,size:4096,added_at:1,seed:false})
        root.phase=1;return
      }
      if(root.phase===1){
        root.check("rendered_roles","list replacement updates NAME and STATE without resort or delegate recreation",labels,labels.indexOf("After rename")>=0 && labels.indexOf("Done")>=0 && labels.indexOf("Before rename")<0 && root.row(table)===root.delegateBefore)
        service.store.apply({id:2,ok:true,result:{sub:1,topics:["torrents"]}},{op:"subscribe"})
        var changed={};changed[root.hash]={name:"Delta rename",state:"downloading",progress:0.75,peers:3}
        service.store.apply({ev:"delta",sub:1,topic:"torrents",rev:1,changed:changed})
        service.store.syncModel(rows,"added",false)
        root.phase=2;return
      }
      root.check("rendered_roles","subscription delta updates the same visible delegate",labels,labels.indexOf("Delta rename")>=0 && labels.indexOf("Downloading")>=0 && labels.indexOf("After rename")<0 && root.row(table)===root.delegateBefore)
      console.log("FBTQS-PASS rendered_roles")
      stop();Quickshell.execDetached(["qs","kill","-p",Quickshell.shellPath(".")])
    }
  }
}
