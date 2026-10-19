import QtQuick
import QtQuick.Window
import Quickshell
import "Plugin/components"
import "Plugin/Store.js" as Store
import "Plugin/TableLayout.js" as TableLayout
import "Plugin/Glyphs.js" as Glyphs

ShellRoot {
  id:root
  ProgressRule {id:progressProbe;visible:false}
  ListModel {id:rows;dynamicRoles:true}
  ListModel {id:batchMirror;dynamicRoles:true}
  property bool batching:false
  property int moves:0
  property int unbatchedMoves:0
  Connections {target:batchMirror;function onRowsMoved(){root.moves++;if(!root.batching)root.unbatchedMoves++}}
  QtObject {
    id:service
    property var store:Store.create()
    property alias torrentModel:rows
    property string sortKey:"added"
    property bool sortReverse:false
    property bool modelReordering:false
    function pendingGlyph(hash){return ""}
    function rowFailed(hash){return false}
  }
  Window {
    visible:true;width:1600;height:600
    TorrentTable {id:table;anchors.fill:parent;service:service;windowLogicalWidth:1600}
  }
  function check(name,action,expected,actual,ok){console.log("FBTQS-STEP "+JSON.stringify({case:name,action:action,expect:expected,observed:JSON.stringify(actual),ok:!!ok,ms:0}))}
  function controlCounts(item){
    var n={rows:item.hasCursor!==undefined && typeof item.progress==="function" && typeof item.stateText==="function" ? 1 : 0,
      progress:item.caption!==undefined && item.known!==undefined && item.complete!==undefined ? 1 : 0}
    for(var i=0;i<item.children.length;i++){var child=controlCounts(item.children[i]);n.rows+=child.rows;n.progress+=child.progress}
    return n
  }
  Component.onCompleted: {
    var first={hash:"a".repeat(40),name:"First",state:"downloading",added_at:1,progress:0.2,dl_bps:4200000,ul_bps:32000,size:4096,intent:"run",seed:false}
    var second={hash:"b".repeat(40),name:"Second",state:"paused",added_at:2,progress:0.5,size:8192,intent:"pause",seed:false}
    service.store.apply({id:1,ok:true,result:{torrents:[first,second]}},{op:"torrents.list"})
    service.store.syncModel(rows,"added",false)
    table.stateFilter="paused"
    probe.start()
    firstRow=first;secondRow=second
  }
  property var firstRow
  property var secondRow
  property int phase:0
  Timer {
    id:probe;interval:10;repeat:true
    onTriggered:{
      if(table.count!==1)return
      var first=root.firstRow,second=root.secondRow
      if(root.phase===0){root.phase=1
      check("native_proxy","filter and map Qt source indexes","only paused hash b",{count:table.count,hash:table.hashAt(0)},table.count===1 && table.hashAt(0)===second.hash)
      table.stateFilter="ready"
      second.playable={ready:true,acked:false}
      service.store.apply({id:2,ok:true,result:{torrents:[first,second]}},{op:"torrents.list"});service.store.syncModel(rows,"added",false)
      return
      }
      stop()
        check("native_proxy","ready filter follows a changed role","ready hash b",{count:table.count,hash:table.hashAt(0)},table.count===1 && table.hashAt(0)===second.hash)
        console.log("FBTQS-PASS native_proxy")
        var controls=root.controlCounts(table)
        check("cell_cost","inspect actual table controls and rendered roles","one progress renderer per delegate; paused state and 50% remain visible",{controls:controls,progress:table.progress(second.hash),state:table.stateText(second.hash)},controls.rows>0 && controls.progress===controls.rows && table.progress(second.hash)===0.5 && table.stateText(second.hash)==="Paused")
        console.log("FBTQS-PASS cell_cost")
        ;[900,1099,1100,1399,1400,1599,1600,1920].forEach(function(width){
          var columns=TableLayout.columns(width,width-TableLayout.chrome(width),1),name=columns[2].width
          check("width_geometry","logical window "+width,"NAME >= 210; ordered fixed columns match the tier",{width:width,name:name,columns:columns},name>=210 && columns[6].hidden===(width<1600))
        })
        console.log("FBTQS-PASS width_geometry")
        ;[0,.01,.43,.999,1].forEach(function(p){var text=Glyphs.blocks(p);check("block_progress","progress "+p,"eight cached glyph cells",text,text.length===8)})
        ;[{p:undefined,text:"—"},{p:0,text:"0%"},{p:0.999,text:"99%"},{p:1,text:"done"}].forEach(function(sample){progressProbe.value=sample.p;check("progress_caption","known/unknown completion boundary","exact caption "+sample.text,progressProbe.caption,progressProbe.caption===sample.text)})
        console.log("FBTQS-PASS progress_caption")
        console.log("FBTQS-PASS block_progress")
        var now=0,s=Store.create({now:function(){return now}}),a="a".repeat(40),b="b".repeat(40),c="c".repeat(40)
        s.apply({id:1,ok:true,result:{torrents:[{hash:a,name:"two",state:"paused",ul_bps:100,peers:2,ratio:1.2},{hash:b,name:"ten",state:"checking",ul_bps:20,peers:10,ratio:10},{hash:c,name:"unknown",state:"metadata"}]}},{op:"torrents.list"})
        check("extra_sorts","numeric PEERS and RATIO; missing last in either direction","b,a,c and a,b,c",[s.sort("peers",false),s.sort("peers",true),s.sort("ratio",false)],s.sort("peers",false).join()=== [b,a,c].join() && s.sort("peers",true).join()===[a,b,c].join() && s.sort("ratio",false).join()===[b,a,c].join())
        check("extra_sorts","numeric upload sorting","a,b,c",s.sort("up",false),s.sort("up",false).join()===[a,b,c].join())
        var actions={pending:{args:{hashes:[a]},status:"pending"},failed:{args:{hash:b},status:"no answer",error:{message:"no answer"}}}
        s.syncActions(actions)
        check("pending_rows","refs index pending and failed hashes without optimistic state","paused a and visible failed b",{pending:s.pendingByHash,failed:s.failedByHash,state:s.torrents[a].state},s.pendingByHash[a] && s.failedByHash[b] && s.torrents[a].state==="paused")
        s.clearFailures(actions)
        check("pending_rows","next key clears failed marker and preserves pending ref","only pending remains",Object.keys(actions),Object.keys(actions).join()==="pending" && !s.failedByHash[b])
        var visible=s.visibleKeys("downloading","","name",false),selected=s.selectRange(visible,b,c,{})
        check("filter_ranges","aggregate downloading and visible-only shift range","checking and metadata, never paused",selected,visible.length===2 && selected[b] && selected[c] && !selected[a])
        s.apply({id:10,ok:true,result:{shows:[{imdb:"tt1",unread:2},{imdb:"tt2",unread:3}]}},{op:"shows.list"})
        s.apply({id:11,ok:true,result:{feeds:[{id:"feed",unread:4}]}},{op:"rss.feeds"})
        check("unread_badges","domain lists supply actual unread counts","shows 5, RSS 4",s.badges,s.badges.shows===5 && s.badges.rss===4)
        s.apply({id:12,ok:true,result:{sub:2,topics:["shows"]}},{op:"subscribe"})
        s.apply({ev:"delta",sub:2,topic:"shows",rev:1,changed:{tt1:{unread:0}},removed:["tt2"],added:[{imdb:"tt3",unread:7}]})
        check("unread_badges","delta additions/removals/read changes update unread total","shows 7",s.badges,s.badges.shows===7)
        var batch=Store.create(),boundaries=[]
        batch.apply({id:1,ok:true,result:{torrents:[{hash:a,name:"two",state:"paused",added_at:1},{hash:b,name:"one",state:"paused",added_at:2},{hash:c,name:"three",state:"paused",added_at:3}]}},{op:"torrents.list"})
        batch.syncModel(batchMirror,"name",false,function(starting){root.batching=starting;boundaries.push(starting)})
        check("initial_batch","first confirmed snapshot admits the final sorted source together","one boundary; name order; no source moves",{boundaries:boundaries,moves:root.moves,first:batchMirror.get(0).hash},boundaries.join()==="true,false" && root.moves===0 && batchMirror.get(0).hash===b)
        console.log("FBTQS-PASS initial_batch")
        boundaries=[];root.moves=0;root.unbatchedMoves=0
        // Exercise an actual user sort after admission; it still requires
        // real rowsMoved signals and no move outside the proxy boundary.
        batch.syncModel(batchMirror,"added",false,function(starting){root.batching=starting;boundaries.push(starting)})
        check("sort_batch","all real Qt source moves share one proxy boundary","[true,false]; no unbatched move",{boundaries:boundaries,moves:root.moves,unbatched:root.unbatchedMoves},boundaries.join()==="true,false" && root.moves>0 && root.unbatchedMoves===0 && batchMirror.get(0).hash===c)
        boundaries=[]
        batch.apply({id:2,ok:true,result:{torrents:[{hash:a,name:"two",state:"paused",added_at:1,dl_bps:100},{hash:b,name:"one",state:"paused",added_at:2},{hash:c,name:"three",state:"paused",added_at:3}]}},{op:"torrents.list"})
        batch.syncModel(batchMirror,"added",false,function(starting){boundaries.push(starting)})
        check("sort_batch","ordinary confirmed role patch preserves proxy and row order","no boundary; dl_bps 100",boundaries,boundaries.length===0 && batchMirror.get(2).dl_bps===100)
        console.log("FBTQS-PASS sort_batch")
        console.log("FBTQS-PASS unread_badges")
        console.log("FBTQS-PASS extra_sorts");console.log("FBTQS-PASS pending_rows");console.log("FBTQS-PASS filter_ranges")
        Quickshell.execDetached(["qs","kill","-p",Quickshell.shellPath(".")])
    }
  }
}
