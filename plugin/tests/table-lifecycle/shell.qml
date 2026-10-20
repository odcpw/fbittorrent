import QtQuick
import QtQuick.Window
import Quickshell
import "Plugin/components"
import "Plugin/Store.js" as Store

ShellRoot {
  id:root
  property var a:({hash:"a".repeat(40),name:"Watched one",state:"done",staged:false,progress:1,added_at:1})
  property var b:({hash:"b".repeat(40),name:"Watched two",state:"done",staged:false,progress:1,added_at:1})
  property var c:({hash:"c".repeat(40),name:"Unrelated film",state:"paused",staged:false,progress:0,added_at:1})
  property var review:({hash:"d".repeat(40),name:"Search release",state:"paused",staged:true,added_at:4})
  property var cases:[]
  property int step:0
  ListModel {id:rows;dynamicRoles:false}
  QtObject {
    id:svc
    property var store:Store.create()
    property alias torrentModel:rows
    property string sortKey:"added"
    property bool sortReverse:false
    property bool modelReordering:false
    signal rowOrderChanging()
    signal rowOrderChanged()
    function pendingGlyph(hash){return ""}
    function rowFailed(hash){return false}
  }
  Window {visible:true;width:1600;height:650;TorrentTable {id:table;anchors.fill:parent;service:svc;windowLogicalWidth:1600}}
  function snapshot(torrents){
    svc.store.apply({id:1,ok:true,result:{torrents:torrents}},{op:"torrents.list"})
    svc.store.syncModel(rows,svc.sortKey,svc.sortReverse,function(start){if(start)svc.rowOrderChanging();svc.modelReordering=start;if(!start)svc.rowOrderChanged()})
  }
  function texts(item){var out=[];if(typeof item.text==="string" && item.visible)out.push(item.text);for(var i=0;i<item.children.length;i++)out=out.concat(texts(item.children[i]));return out}
  function rendered(item){
    if(item.hasCursor!==undefined && item.torrent!==undefined)return item.visible ? [{hash:item.torrent.hash,texts:texts(item)}] : []
    var out=[];for(var i=0;i<item.children.length;i++)out=out.concat(rendered(item.children[i]));return out
  }
  function matches(expected){
    var actual=rendered(table)
    return actual.length===expected.length && expected.every(function(wanted){var same=actual.filter(function(row){return row.hash===wanted.hash});return same.length===1 && same[0].texts.indexOf(wanted.name)>=0 && same[0].texts.indexOf(wanted.staged ? "Reviewing" : wanted.state==="done" ? "Done" : wanted.state==="paused" ? "Paused" : "Downloading")>=0})
  }
  function verdict(name,expected){
    var ok=matches(expected),actual=rendered(table)
    if(name==="search_commit")ok=ok && table.progress(review.hash)===0.25 && actual.every(function(row){return row.texts.indexOf("Reviewing")<0})
    console.log("FBTQS-STEP "+JSON.stringify({case:name,action:"apply row removal/reinsertion through the real Qt proxy",expect:"each rendered hash has exactly its current name/state and known progress",observed:JSON.stringify({rows:actual,progress:table.progress(review.hash)}),ok:ok,ms:0}))
    console.log("FBTQS-PASS "+name)
  }
  Component.onCompleted:{
    // First-ever metadata-unknown row: Discard then re-add the same hash before
    // metadata/commit patches it. The table stays alive while Search is open.
    // Equal added_at values also force native source moves on single Undo.
    cases=[
      {rows:[review]},
      {rows:[]},
      {rows:[review]},
      {rows:[Object.assign({},review,{staged:false,state:"downloading",progress:0.25,size:8388608})],check:"search_commit"},
      {rows:[]},
      {rows:[a,b,c]},
      {rows:[b,c]},
      {rows:[a,b,c],check:"undo_one"},
      {rows:[c]},
      {rows:[c,b]},
      {rows:[c,b,a],check:"undo_bulk"}
    ];clock.start()
  }
  Timer {id:clock;interval:60;repeat:true;onTriggered:{
    if(root.step>0){var prior=root.cases[root.step-1];if(prior.check)root.verdict(prior.check,prior.rows)}
    if(root.step===root.cases.length){stop();Quickshell.execDetached(["qs","kill","-p",Quickshell.shellPath(".")]);return}
    root.snapshot(root.cases[root.step++].rows)
  }}
}
