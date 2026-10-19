import QtQuick
import Quickshell
import "Plugin/components" as Components
ShellRoot {
  id:root
  property int phase:0
  property string chosen:""
  property bool cancelled:false
  readonly property string initial:Quickshell.shellPath("folders")
  function check(expect,ok){console.log("FBTQS-STEP "+JSON.stringify({case:"folders",action:"exercise actual Qt directory model and picker",expect:expect,observed:JSON.stringify(picker.state()),ok:!!ok,ms:0}))}
  function verify(){
    var s=picker.state()
    if(s.loading)return
    if(phase===0 && s.count===2){
      check("only visible directories are listed",s.names[0]==="a child #1" && s.names[1]==="z child")
      picker.move(1);picker.handleKey({key:Qt.Key_Return,text:""})
      check("Enter selects the highlighted folder as an absolute path",chosen===initial+"/a child #1")
      check("relative and control-containing paths are refused",!picker.navigate("relative") && !picker.navigate("/tmp/a\u0000b"))
      phase=1;picker.navigate(chosen)
    }else if(phase===1 && s.count===1){
      check("spaces and URL fragment characters retain the real directory",s.path===chosen && s.names[0]==="inner")
      phase=2;picker.handleKey({key:Qt.Key_Left,text:""})
    }else if(phase===2 && s.count===2){
      check("Left returns to the parent and resets the cursor",s.path===initial && s.cursor===-1)
      picker.handleKey({key:Qt.Key_Escape,text:""});check("Escape cancels without selecting another path",cancelled && chosen===initial+"/a child #1")
      phase=3;console.log("FBTQS-PASS folders")
      Qt.callLater(function(){Quickshell.execDetached(["qs","kill","-p",Quickshell.shellPath(".")])})
    }
  }
  Components.FolderPicker {id:picker;width:560;height:360;initialPath:root.initial;onSelected:function(path){root.chosen=path};onCancelled:root.cancelled=true;onChanged:Qt.callLater(root.verify)}
  Component.onCompleted:Qt.callLater(verify)
}
