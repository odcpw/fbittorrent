import QtQuick
import QtQuick.Window
import Quickshell
import Quickshell.Io
import "Plugin"
import "Plugin/components"

ShellRoot {
  id:root
  Service {id:svc;settings:({binary:Quickshell.env("FBT_TEST_BINARY")})}
  Dashboard {id:dashboard;service:svc}
  Window {id:probe;visible:true;width:640;height:80;RateStatus {id:rates;anchors.fill:parent;service:svc}}
  IpcHandler {
    target:"telemetry"
    function state():string{return JSON.stringify({online:svc.online,history:rates.history,rates:rates.rateText,visible:dashboard.svc.dashboardVisible,topics:svc.client ? svc.client.state().subs : {}})}
    function open(view:string):bool{return dashboard.open(JSON.stringify({view:view}))}
    function hide():bool{dashboard.close();return true}
    function grab(path:string):bool{return dashboard.grab(path)}
  }
}
