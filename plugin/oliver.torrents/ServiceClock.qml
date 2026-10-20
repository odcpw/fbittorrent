import QtQuick

// No heartbeat: pushed state drives rendering. The one-shot clock preserves
// request timeouts and the few UI transitions that actually have a deadline.
Item {
  id:root
  required property var service
  required property var timer
  function schedule(){Qt.callLater(arm)}
  function arm(){
    var s=service,now=Date.now(),at=s.client ? s.client.nextWakeup() : Infinity
    s.now=now
    if(s.addFlow)at=Math.min(at,s.addFlow.nextWakeup())
    if(s.settingsFlow)at=Math.min(at,s.settingsFlow.nextWakeup())
    if(s.dashboardVisible && s.cleanupFlow && s.cleanupFlow.until>now)at=Math.min(at,s.cleanupFlow.until)
    if(s.client && !s.online && s.client.mode==="connecting" && s.client.started+2000>now)at=Math.min(at,s.client.started+2000)
    if(at===timer.at && timer.running)return
    timer.stop();timer.at=at
    if(Number.isFinite(at)){timer.interval=Math.max(1,Math.ceil(at-now));timer.start()}
  }
  function wake(){
    var s=service,now=Date.now();s.now=now
    if(s.client && s.client.nextWakeup()<=now)s.client.tick()
    if(s.addFlow && s.addFlow.nextWakeup()<=now)s.addFlow.tick()
    if(s.settingsFlow && s.settingsFlow.nextWakeup()<=now)s.settingsFlow.tick()
    schedule()
  }
  Connections {
    target:root.service
    function onAddRevisionChanged(){root.schedule()}
    function onSettingsRevisionChanged(){root.schedule()}
    function onCleanupRevisionChanged(){root.schedule()}
    function onDashboardVisibleChanged(){root.schedule()}
    function onClientChanged(){root.schedule()}
  }
}
