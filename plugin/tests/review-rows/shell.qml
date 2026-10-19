import QtQuick
import QtQuick.Window
import Quickshell
import "Plugin/components"
import "Plugin/Store.js" as Store
import "Plugin/Model.js" as Model
import "Plugin/DetailText.js" as DetailText

ShellRoot {
  id:root
  property int phase:0
  property int waiting:0
  readonly property string saved:"a".repeat(40)
  readonly property string staged:"b".repeat(40)
  ListModel {id:rows;dynamicRoles:true}
  QtObject {
    id:service
    property var store:Store.create()
    property alias torrentModel:rows
    property var status:({loaded:false})
    property string sortKey:"added"
    property bool sortReverse:false
    property var badgeCounts:({})
    function pendingGlyph(hash){return ""}
    function rowFailed(hash){return false}
  }
  Window {visible:true;width:1600;height:500
    TorrentFilters {id:filters;width:180;service:service}
    TorrentTable {id:table;x:200;width:1400;height:parent.height;service:service;windowLogicalWidth:1600}
  }
  function apply(frame,request){service.store.apply(frame,request);service.store.syncModel(rows,"added",false);service.status=Model.apiStatus(service.store,true)}
  function texts(item){var out=[];if(typeof item.text==="string" && item.visible)out.push(item.text);for(var i=0;i<item.children.length;i++)out=out.concat(texts(item.children[i]));return out}
  function check(expect,actual,ok){console.log("FBTQS-STEP "+JSON.stringify({case:"review_rows",action:"render staged/committed membership through the production reducer",expect:expect,observed:JSON.stringify(actual),ok:!!ok,ms:0}))}
  Component.onCompleted:{
    // Daemon summary intentionally excludes staged entries from active totals.
    service.store.summary={counts:{total:1,paused:1,staged:1}}
    apply({id:1,ok:true,result:{torrents:[{hash:saved,name:"Saved",state:"paused",staged:false},{hash:staged,name:"Review magnet",state:"paused",staged:true,playable:{ready:true,acked:false}}]}},{op:"torrents.list"})
    apply({id:2,ok:true,result:{sub:1,topics:["torrents"]}},{op:"subscribe"})
  }
  Timer {interval:50;repeat:true;running:true
    onTriggered:{
      var shown=root.texts(table),captions=root.texts(filters),c=service.store.counts
      // Qt's proxy/delegate updates are asynchronous. Wait for the actual
      // membership, with a bounded failure, rather than assuming one 50 ms tick.
      var settled=root.phase===0 ? table.count===2 && shown.indexOf("Reviewing")>=0 : root.phase===1 ? table.count===1 && table.hashAt(0)===root.saved : root.phase===2 ? table.count===1 && table.hashAt(0)===root.staged : root.phase===3 ? table.count===0 : root.phase===4 ? table.count===1 : table.count===0
      if(!settled && root.waiting++<60)return
      root.waiting=0
      if(root.phase===0){
        root.check("All 2 / Paused 1 / Reviewing 1 describe the two actual rows",{rows:table.count,labels:captions,c:c,status:service.status},table.count===2 && filters.count("all")===2 && filters.count("paused")===1 && filters.count("reviewing")===1 && captions.indexOf("All  2")>=0 && captions.indexOf("Paused  1")>=0 && captions.indexOf("Reviewing  1")>=0 && service.status.total===1)
        root.check("staged row says Reviewing and contributes no Ready/Paused transfer state",shown,shown.indexOf("Reviewing")>=0 && c.ready===0 && service.store.filter("paused").length===1 && !Model.isStopped(service.store.torrents[root.staged]))
        table.stateFilter="paused";root.phase++;return
      }
      if(root.phase===1){root.check("Paused shows only the committed paused record",table.count,table.count===1 && table.hashAt(0)===root.saved);table.stateFilter="reviewing";root.phase++;return}
      if(root.phase===2){
        root.check("Reviewing shows only the staged record",shown,table.count===1 && table.hashAt(0)===root.staged && shown.indexOf("Reviewing")>=0)
        var changed={};changed[root.staged]={staged:false};root.apply({ev:"delta",sub:1,topic:"torrents",rev:1,changed:changed});root.phase++;return
      }
      if(root.phase===3){
        root.check("commit moves the same row from Reviewing into Paused without changing All",{c:c,rows:table.count,labels:captions},table.count===0 && c.total===2 && c.paused===2 && c.reviewing===0 && c.ready===1 && rows.get(service.store.modelIndex(root.staged)).uiReady && captions.indexOf("Paused  2")>=0 && service.store.visibleKeys("paused","","added",false).length===2)
        root.apply({ev:"delta",sub:1,topic:"torrents",rev:2,added:[{hash:"c".repeat(40),name:"Resolving",state:"metadata",staged:true}]});root.phase++;return
      }
      if(root.phase===4){
        root.check("metadata resolution while reviewing is not counted as downloading",{c:c,rows:table.count},table.count===1 && c.total===3 && c.reviewing===1 && c.metadata===0 && filters.count("downloading")===0)
        root.apply({ev:"delta",sub:1,topic:"torrents",rev:3,removed:["c".repeat(40)]});root.phase++;return
      }
      root.check("discard removes only the review row and its count",{c:c,rows:table.count},table.count===0 && c.total===2 && c.reviewing===0 && c.staged===0 && c.paused===2)
      root.check("Details Upload label follows effective permission rather than completion seed opt-in",[DetailText.policy({seed:true,upload_enabled:false}),DetailText.policy({seed:false,upload_enabled:true})],DetailText.policy({seed:true,upload_enabled:false})==="Upload: off" && DetailText.policy({seed:false,upload_enabled:true})==="Upload: on")
      service.store.summary={counts:{total:1,downloading:1}};service.status=Model.apiStatus(service.store,true)
      root.check("bar status still uses fresh daemon summary when cached table rows are no longer sampled",service.status,service.status.total===1 && service.status.active===1)
      console.log("FBTQS-PASS review_rows");stop();Quickshell.execDetached(["qs","kill","-p",Quickshell.shellPath(".")])
    }
  }
}
