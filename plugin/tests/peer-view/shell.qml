import QtQuick
import Quickshell
import "Plugin/views"
import "Plugin/Model.js" as Model
import "Plugin/PopupState.js" as PopupState
import "Plugin/BarState.js" as BarState
ShellRoot {
  id:root
  PeerRow {id:row;width:350;peer:({addr:"[::1]:42001",client:"Fixture client",conn:"tcp_mse",incoming:false,state:"live",seed:true,dl_bps:32768,ul_bps:0,downloaded:65536,uploaded:0})}
  function check(name,ok,expect,observed){console.log("FBTQS-STEP "+JSON.stringify({case:name,action:"exercise actual peer row and count formatting",expect:expect,observed:JSON.stringify(observed),ok:!!ok,ms:0}))}
  Component.onCompleted:{
    var rendered=row.rendered()
    check("peer_row",["[::1]:42001","Fixture client","OUT","TCP / MSE","live","seed yes","↓","↑","Downloaded","uploaded"].every(function(s){return rendered.indexOf(s)>=0}),"all API peer fields appear in plain text",rendered)
    row.peer={addr:"127.0.0.1:42002",conn:"tcp",incoming:true,state:"live",downloaded:0,uploaded:0}
    rendered=row.rendered()
    check("peer_row",rendered.indexOf("IN · TCP · live · seed —")>=0 && rendered.indexOf("↓ — · ↑ —")>=0,"absent client, rates and bitfield-derived seed status stay unknown",rendered)
    var known={state:"downloading",peers:2,seeds:1},unknown={state:"metadata"},zero={peers:0,seeds:0}
    check("counts",Model.peerCount(known,true)==="1/2" && Model.peerCount(unknown,true)==="—/—" && Model.peerCount(zero,true)==="0/0","compact seeds/connected peers distinguish unknown and known zero",[Model.peerCount(known,true),Model.peerCount(unknown,true),Model.peerCount(zero,true)])
    check("counts",Model.peerCount({peers:2},false)==="2 peers connected · seeds unknown" && PopupState.meta(known,false).indexOf("1 seeds / 2 peers connected")>=0,"popup includes actual connected counts without fabricating seeds",PopupState.meta(known,false))
    var store={summary:{counts:{downloading:1},top:[{name:"Current",peers:2,seeds:1}]},torrents:{stale:{name:"Stale cached row",peers:99,seeds:99}},info:{}}
    var svc={online:true,backendState:"running",status:{dl:0,up:0},client:{caps:[]},store:store}
    var bar=BarState.describe(svc)
    check("bar_counts",bar.tooltip.indexOf("Current: 1 seeds / 2 peers connected")>=0 && bar.tooltip.indexOf("99")<0,"bar uses current bounded summary observations, not stale detail rows",bar)
    store.summary.top=[{name:"Unknown"}];bar=BarState.describe(svc)
    check("bar_counts",bar.tooltip.indexOf("peers")<0,"bar omits counts that the current summary cannot observe",bar)
    ;["peer_row","counts","bar_counts"].forEach(function(name){console.log("FBTQS-PASS "+name)})
    Qt.callLater(function(){Quickshell.execDetached(["qs","kill","-p",Quickshell.shellPath(".")])})
  }
}
