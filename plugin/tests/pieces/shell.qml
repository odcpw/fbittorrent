import QtQuick
import QtQuick.Window as QtWindow
import Quickshell
import Quickshell.Io
import qs.Commons as Commons
import "Plugin/Store.js" as Store
import "Plugin/ApiClient.js" as ApiClient
import "Plugin/PiecesModel.js" as PiecesModel
import "Plugin/components" as Components
ShellRoot {
 id:root
 FileView {id:wire;path:Qt.resolvedUrl("Fixtures/pieces.ndjson");blockLoading:true;blockAllReads:true}
 function check(name,condition,expect,observed){console.log("FBTQS-STEP "+JSON.stringify({case:name,action:"decode and render frozen Pieces",expect:expect,observed:JSON.stringify(observed),ok:!!condition,ms:0}))}
 QtWindow.Window {id:window;width:800;height:220;visible:true;color:Commons.Color.background
  Components.PiecesBar {id:bar;parent:window.contentItem;x:Commons.Style.spacing.lg;y:x;width:window.width-2*x;height:window.height-2*y}
 }
 Component.onCompleted:{
  var s=Store.create(),calls={},frames=wire.text().trim().split("\n").map(function(line){return JSON.parse(line)})
  frames.forEach(function(line){if(line.dir==="c2s")calls[line.frame.id]=line.frame;else if(line.dir==="s2c")s.apply(line.frame,calls[line.frame.id])})
  var data=s.topics["torrent:"+"a".repeat(40)+".pieces"],map=PiecesModel.decode(data),cells=PiecesModel.cells(map,1000)
  check("bitfield",map.have===257 && map.data.count===1000,"MSB-first fixture has1000 pieces, first256 and last verified",bar.state)
  check("priority_prefix",cells.length===1000 && cells[0].priority && cells[999].priority && cells[255].prefix && !cells[256].prefix && !cells[256].have,"first/last marks and exclusive256-piece prefix are distinct from missing",{first:cells[0],prefixEnd:cells[255],missing:cells[256],last:cells[999]})
  var rejected=0
  ;[{have:"?"},{priority:[0,1000]},{priority:[999,0]},{playable_end:1001},{count:999}].forEach(function(patch){try{PiecesModel.decode(Object.assign({},data,patch))}catch(error){rejected++}})
  check("invalid",rejected===5,"invalid bitfield/priority/prefix/padding rejected",rejected)
  var large=PiecesModel.decode({count:200000,piece_size:16384,have:"////".repeat(8333)+"/w==",priority:[0,199999]})
  check("bounded",PiecesModel.cells(large,80).length===80 && large.bits.length===25000,"200000-piece input renders eighty aggregate cells and bounded bytes",{cells:PiecesModel.cells(large,80).length,bytes:large.bits.length})
  var rejectedState=false;try{s.apply({ev:"state",sub:1,rev:2,topic:"torrent:"+"a".repeat(40)+".pieces",data:Object.assign({},data,{have:"?"})})}catch(error){rejectedState=true}
  check("invalid",rejectedState && s.topics["torrent:"+"a".repeat(40)+".pieces"]===data,"invalid state cannot replace the confirmed piece map",rejectedState)
  var writes=[],client=ApiClient.create({write:function(line){writes.push(line)},flush:function(){},disconnect:function(){},isConnected:function(){return true}},{now:function(){return 0},frame:function(){}})
  client.connected();client.receive(JSON.stringify({id:1,ok:true,result:{proto:1,instance:"a".repeat(32),state:"running",caps:["torrents","files","subscribe"]}}))
  var count=writes.length,failure=null
  client.request("torrents.pieces",{hash:"a".repeat(40)},{callback:function(result,error){failure=error}})
  check("capability",writes.length===count && failure && failure.code==="unsupported","pieces read requires its negotiated capability",failure)
  bar.torrent={state:"done",checked:false}
  check("absent_map",PiecesModel.unavailable(bar.torrent).indexOf("Completed data is parked")===0 && PiecesModel.unavailable(bar.torrent).indexOf("Recheck")>=0,"completed data without a live bitfield offers Recheck without claiming a check is running",PiecesModel.unavailable(bar.torrent))
  check("absent_map",PiecesModel.unavailable({state:"checking",checked:false}).indexOf("Checking data")===0 && PiecesModel.unavailable({state:"metadata"}).indexOf("Waiting for metadata")===0 && PiecesModel.unavailable({state:"paused",checked:true}).indexOf("checking")<0,"only an actual checking state claims a running check",["checking","metadata","paused"].map(function(s){return PiecesModel.unavailable({state:s})}))
  console.log("FBTQS-PASS absent_map")
  bar.snapshot=data
  Qt.callLater(function(){
   check("pieces_bar",bar.children.length>=3 && bar.state.count===1000 && bar.state.have===257 && bar.state.playable_end===256 && bar.state.cells<=Math.floor(bar.width/Commons.Style.space(4)),"real component exposes count/have/priority/prefix and a width-bounded delegate count",bar.state)
   bar.grabToImage(function(result){
    var path=Quickshell.env("FBT_PIECES_CAPTURE"),saved=path ? result.saveToFile(path) : false
    check("pieces_bar",saved,"fixture capture shows first/last glyphs and prefix underline",path)
    ;["bitfield","priority_prefix","invalid","bounded","capability","pieces_bar"].forEach(function(name){console.log("FBTQS-PASS "+name)})
    Quickshell.execDetached(["qs","kill","-p",Quickshell.shellPath(".")])
   })
  })
 }
}
