import QtQuick
import Quickshell
import "Plugin/views"
ShellRoot {
  id:root
  property var edits:[]
  property var controller:({editTracker:function(url,add,done){root.edits.push({url:url,add:add});return true}})
  TrackerEditor {id:editor;width:400;height:400;flow:root.controller}
  function find(item,name){if(item.objectName===name)return item;for(var i=0;i<item.children.length;i++){var child=find(item.children[i],name);if(child)return child}return null}
  function check(name,ok,expect,observed){console.log("FBTQS-STEP "+JSON.stringify({case:name,action:"exercise actual tracker editor",expect:expect,observed:observed,ok:!!ok,ms:0}))}
  Component.onCompleted:{
    editor.show(false,"https://tracker.invalid/SYNTHETIC_PATH_PASSKEY/announce#fbt-tracker=opaque-identity")
    check("privacy",editor.confirmationText.indexOf("SYNTHETIC_PATH_PASSKEY")<0 && editor.confirmationText.indexOf("opaque-identity")<0,"Remove confirmation exposes only the redacted endpoint",editor.confirmationText)
    editor.close()
    editor.show(false,"https://private-user:private-password@tracker.invalid/a?passkey=private-token")
    check("cancel",editor.cancelFocused,"remove starts with Cancel selected",editor.cancelFocused)
    editor.key({key:Qt.Key_Return});check("cancel",edits.length===0 && !editor.opened,"Enter cancels without a mutation",edits.length)
    editor.show(false,"https://tracker.invalid/a?passkey=private-token")
    editor.key({key:Qt.Key_Right});editor.key({key:Qt.Key_Return})
    check("confirm",edits.length===1 && !edits[0].add && edits[0].url.endsWith("private-token"),"explicit selection removes the exact private URL identity",edits.length)
    editor.show(true,"");var input=find(editor,"tracker-url-input");input.text="https://tracker.invalid/b?passkey=private-token"
    check("privacy",input.password && input.displayText.indexOf("private-token")<0,"typed URL uses the actual password text field",input.displayText)
    editor.submit();check("confirm",edits.length===2 && edits[1].add,"Add dispatches through the same tracker edit flow",edits.length)
    ;["cancel","confirm","privacy"].forEach(function(name){console.log("FBTQS-PASS "+name)})
    Qt.callLater(function(){Quickshell.execDetached(["qs","kill","-p",Quickshell.shellPath(".")])})
  }
}
