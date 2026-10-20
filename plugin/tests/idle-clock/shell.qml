import QtQuick
import Quickshell
import "Plugin"

ShellRoot {
  id:root
  property int phase:0
  property real protocolAt:Infinity
  property real addAt:Infinity
  property real settingsAt:Infinity
  property int protocolTicks:0
  property int addTicks:0
  property int settingsTicks:0
  QtObject {
    id:service
    property real now:0
    property bool online:true
    property bool dashboardVisible:false
    property int addRevision:0
    property int settingsRevision:0
    property int cleanupRevision:0
    property var cleanupFlow:({until:0})
    property var client:({nextWakeup:function(){return root.protocolAt},tick:function(){root.protocolTicks++;root.protocolAt=Infinity}})
    property var addFlow:({nextWakeup:function(){return root.addAt},tick:function(){root.addTicks++;root.addAt=Infinity}})
    property var settingsFlow:({nextWakeup:function(){return root.settingsAt},tick:function(){root.settingsTicks++;root.settingsAt=Infinity}})
  }
  Timer {id:deadline;property real at:Infinity;onTriggered:clock.wake()}
  ServiceClock {id:clock;service:service;timer:deadline}
  function check(name,ok,value){console.log("FBTQS-STEP "+JSON.stringify({case:name,action:"drive one-shot service deadlines",expect:name,observed:String(value),ok:!!ok,ms:0}))}
  Timer {
    id:step;interval:100
    onTriggered:{
      if(root.phase===0){
        check("quiet",root.protocolTicks+root.addTicks+root.settingsTicks===0,root.protocolTicks)
        root.protocolAt=Date.now()+30;clock.schedule()
      }else if(root.phase===1){
        check("protocol",root.protocolTicks===1 && !root.addTicks && !root.settingsTicks,root.protocolTicks)
        root.addAt=Date.now()+30;service.addRevision++
      }else if(root.phase===2){
        check("add",root.addTicks===1 && root.protocolTicks===1,root.addTicks)
        root.settingsAt=Date.now()+30;service.settingsRevision++
      }else if(root.phase===3){
        check("settings",root.settingsTicks===1,root.settingsTicks)
        service.dashboardVisible=true;service.cleanupFlow={until:Date.now()+30};service.cleanupRevision++
      }else if(root.phase===4){
        check("undo_expiry",service.now>=service.cleanupFlow.until,service.now-service.cleanupFlow.until)
        root.protocolAt=Date.now()+1000;clock.schedule()
        root.protocolAt=Infinity;clock.schedule()
        step.interval=1150
      }else{
        check("cancel",root.protocolTicks===1 && root.addTicks===1 && root.settingsTicks===1,root.protocolTicks)
        ;["quiet","protocol","add","settings","undo_expiry","cancel"].forEach(function(name){console.log("FBTQS-PASS "+name)})
        Qt.quit();return
      }
      root.phase++;step.restart()
    }
  }
  Component.onCompleted:{clock.schedule();step.start()}
}
