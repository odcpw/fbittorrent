import QtQuick
import QtQuick.Window
import Quickshell
import "Plugin/components"
import "Plugin/Model.js" as Model

ShellRoot {
  id:root
  QtObject {id:svc;property bool online:true;property var status:({dl:65536,up:1024,history:[]})}
  Window {visible:true;width:500;height:100;RateStatus {id:rates;anchors.fill:parent;service:svc}}
  Sparkline {id:plot;width:180;height:24}
  function check(action,ok,value){console.log("FBTQS-STEP "+JSON.stringify({case:"history_render",action:action,expect:"bounded received samples and immediate data-driven geometry",observed:JSON.stringify(value),ok:!!ok,ms:0}))}
  Timer {id:finish;interval:30;onTriggered:{root.check("visible chart fits the row at compact spacing scales",rates.implicitHeight>=24 && rates.height>=24,{implicit:rates.implicitHeight,height:rates.height});console.log("FBTQS-PASS history_render");Quickshell.execDetached(["qs","kill","-p",Quickshell.shellPath(".")])}}
  Component.onCompleted:{
    var points=[];for(var i=0;i<60;i++)points.push(i*1024)
    var store={summary:{history:points,counts:{},dl_bps:65536,ul_bps:1024},info:{},transfer:{}}
    svc.status=Model.apiStatus(store,true);plot.samples=rates.history
    check("received ramp reaches the shared dashboard/popup status component",rates.history.length===60 && plot.points[59]===60416 && plot.maximum===60416,{count:rates.history.length,max:plot.maximum,rate:rates.rateText})
    var bounded=[];for(i=0;i<700;i++)bounded.push(i)
    plot.samples=bounded;check("chart stays bounded and ordered",plot.points.length===60 && plot.points[0]===640 && plot.points[59]===699,plot.points)
    plot.samples=[0,0,0];check("idle data is zero, without invented interpolation",plot.maximum===1 && plot.points.every(function(n){return n===0}),plot.points)
    svc.online=false;check("stopped does not show retained data as live",rates.history.length===0 && rates.rateText==="Stopped",rates.rateText)
    svc.online=true;svc.status={dl:8192,up:0,history:[1,3,8]};check("data replacement updates immediately",rates.history.length===3 && rates.history[2]===8,rates.history)
    finish.start()
  }
}
