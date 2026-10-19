import QtQuick
import QtQuick.Window
import Quickshell
import "Plugin/components"
ShellRoot {
 id:root
 Window {visible:true;width:600;height:150;Statistics {id:stats;anchors.fill:parent}}
 function check(action,ok,value){console.log("FBTQS-STEP "+JSON.stringify({case:"statistics_render",action:action,expect:"confirmed totals, binary units and truthful unknown/offline display",observed:JSON.stringify(value),ok:!!ok,ms:0}))}
 Component.onCompleted:{
  check("unknown totals are not invented zeros",stats.sessionText==="Loading statistics…" && stats.allTimeText==="",stats.sessionText)
  stats.statistics={session:{downloaded:8388608,uploaded:0,started_at:1},all_time:{downloaded:16777216,uploaded:1024,completed:2}}
  check("known real zeros and binary units remain visible",stats.sessionText.indexOf("8.0 MiB")>=0 && stats.sessionText.indexOf("0 B")>=0 && stats.allTimeText.indexOf("16.0 MiB")>=0 && stats.allTimeText.indexOf("2 completed")>=0,{session:stats.sessionText,all:stats.allTimeText})
  stats.statistics={session:{downloaded:0,uploaded:0,started_at:2},all_time:{downloaded:16777216,uploaded:1024,completed:2}}
  check("new session resets while durable totals remain",stats.sessionText.indexOf("0 B")>=0 && stats.allTimeText.indexOf("16.0 MiB")>=0,{session:stats.sessionText,all:stats.allTimeText})
  stats.online=false;check("offline does not pass stale session totals off as current",!stats.known && stats.allTimeText==="",stats.sessionText)
  Qt.callLater(function(){console.log("FBTQS-PASS statistics_render");Quickshell.execDetached(["qs","kill","-p",Quickshell.shellPath(".")])})
 }
}
