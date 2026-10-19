import QtQuick
import QtQuick.Window
import Quickshell
import "Plugin/components"
ShellRoot {
 id:root
 property var calls:[]
 property var svc:({hasCap:function(){return true},mutate:function(op,args,cap,callback){root.calls.push({op:op,args:args,done:callback});return true}})
 Window {visible:true;width:600;height:250;DisplayNameEditor {id:editor;anchors.fill:parent;service:root.svc;torrent:({hash:"a".repeat(40),name:"Original name"})}}
 function field(item){if(item.objectName==="displayNameInput")return item;for(var i=0;i<item.children.length;i++){var f=field(item.children[i]);if(f)return f}return null}
 function check(action,ok,value){console.log("FBTQS-STEP "+JSON.stringify({case:"rename_editor",action:action,expect:"confirmed display-only rename; empty clears; cancellation and validation do not mutate",observed:JSON.stringify(value),ok:!!ok,ms:0}))}
 Component.onCompleted:Qt.callLater(function(){
  editor.begin();var input=field(editor)
  check("begin prefills",editor.editing && input.text==="Original name",input.text)
  input.text="Unsaved";editor.key({key:Qt.Key_Escape});check("cancel leaves store alone",!editor.editing && calls.length===0,calls)
  editor.begin();input.text="My title";input.accepted();check("rename waits for confirmation",editor.editing && editor.pending && calls.length===1 && calls[0].args.name==="My title",calls)
  calls[0].done(null,{message:"disk unavailable"});check("failed save keeps draft",editor.editing && !editor.pending && editor.error==="disk unavailable" && input.text==="My title",editor.error)
  input.text="";input.accepted();calls[1].done({state:"done"},null);check("empty name clears through API",!editor.editing && calls[1].args.name==="",calls)
  editor.begin();input.text="bad\tname";input.accepted();check("controls refused before sending",calls.length===2 && !!editor.error,editor.error)
  editor.begin();input.text="Unsaved for original";editor.torrent={hash:"b".repeat(40),name:"Other title"};check("changing rows cancels the old draft",!editor.editing && calls.length===2,editor.editing)
  console.log("FBTQS-PASS rename_editor");Quickshell.execDetached(["qs","kill","-p",Quickshell.shellPath(".")])
 })
}
