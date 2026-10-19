import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons as Commons
import "Plugin/Glyphs.js" as Glyphs
import "Plugin/Model.js" as Model
import "Plugin/TrackerText.js" as TrackerText

ShellRoot {
  id: root
  FileView { id:input; blockLoading:true; blockAllReads:true; preload:false }
  Component { id:textFactory; Text { font.family:Quickshell.env("FBT_FONT_FAMILY"); font.pixelSize:Commons.Style.font.icon; textFormat:Text.PlainText } }
  function check(role,glyph,word) {
    var item=textFactory.createObject(root,{text:glyph})
    console.log("FBTQS-STEP "+JSON.stringify({case:"glyphs",action:"paint "+role,expect:"paintedWidth > 0 in the shell font",observed:"font="+item.font.family+"; width="+item.paintedWidth+"; word="+word,ok:item.paintedWidth>0,ms:0,artifacts:[]}))
    item.destroy()
  }
  Component.onCompleted: {
    Object.keys(Glyphs.codepoints).forEach(function(role){check(role,Glyphs.glyph(role),role)})
    input.path=Quickshell.shellPath("States.json")
    JSON.parse(input.text()).forEach(function(row){
      check(row.state,String.fromCodePoint(parseInt(row.qml_glyph.slice(2),16)),row.word)
      var actual=row.state.indexOf("tracker.")===0 ? TrackerText.status(row.state.slice(8)) : Model.stateLabel({state:row.state})
      console.log("FBTQS-STEP "+JSON.stringify({case:"glyphs",action:"match state vocabulary "+row.state,expect:row.word,observed:actual,ok:actual===row.word,ms:0,artifacts:[]}))
      if(row.state==="metadata")check("metadata text",row.text_glyph,row.word)
    })
    console.log("FBTQS-PASS glyphs")
    Qt.callLater(function(){Quickshell.execDetached(["qs","kill","-p",Quickshell.shellPath(".")])})
  }
}
