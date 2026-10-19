import QtQuick
import Quickshell
import Quickshell.Io
import "Plugin"
ShellRoot {
 Service {id:svc;settings:({binary:Quickshell.env("FBT_TEST_BINARY")})}
 Dashboard {id:dashboard;service:svc}
 function field(item){if(!item)return null;if(item.objectName==="displayNameInput")return item;for(var i=0;i<item.children.length;i++){var f=field(item.children[i]);if(f)return f}return null}
 IpcHandler {
  target:"rename"
  function state():string{return JSON.stringify({online:svc.online,rows:svc.store.torrents,keys:svc.store.visibleKeys("all","", "name",false),filtered:svc.store.visibleKeys("all","Dashboard title","name",false),detail:svc.detailFlow ? svc.detailFlow.row() : null,editing:dashboard.torrents ? dashboard.torrents.editing : false})}
  function open(hash:string):bool{return dashboard.open(JSON.stringify({view:"torrents",hash:hash}))}
  function rename():bool{return dashboard.torrents.handleKey({text:"N",key:Qt.Key_N,modifiers:Qt.ShiftModifier})}
  function submit(name:string):bool{var input=field(dashboard.torrents.pane);if(!input || !input.visible)return false;input.text=name;input.accepted();return true}
  function hide():bool{dashboard.close();return true}
 }
}
