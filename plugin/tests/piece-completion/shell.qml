import QtQuick
import QtQuick.Window
import Quickshell
import "Plugin/components" as Components

ShellRoot {
  id:root
  property int phase:0
  property int waits:0
  Window {
    width:600;height:180;visible:true
    Components.PiecesBar {
      id:bar;anchors.fill:parent
      snapshot:({count:4,piece_size:16384,have:"oA==",priority:[0,1],playable_end:1})
    }
  }
  function cells(item){
    var out=[]
    if(item.modelData!==undefined && typeof item.modelData.first==="number")out.push(item)
    for(var i=0;i<item.children.length;i++)out=out.concat(cells(item.children[i]))
    return out.sort(function(a,b){return a.modelData.first-b.modelData.first})
  }
  function part(item,name){return item.children.find(function(child){return child.objectName===name})}
  function check(expect,observed,ok){console.log("FBTQS-STEP "+JSON.stringify({case:"priority_completion",action:"render the real PiecesBar",expect:expect,observed:JSON.stringify(observed),ok:!!ok,ms:0}))}
  Timer {
    running:true;repeat:true;interval:50
    onTriggered:{
      var cells=root.cells(bar)
      if(cells.length!==4 && root.waits++<60)return
      if(cells.length!==4){root.check("four actual completion delegates",cells.length,false);stop();Quickshell.execDetached(["qs","kill","-p",Quickshell.shellPath(".")]);return}
      var colors=cells.map(function(cell){return String(root.part(cell,"piece-completion").color)})
      var marks=cells.map(function(cell){return root.part(cell,"piece-priority").text})
      var wanted=root.phase===0 ? [true,false,true,false] : [false,true,true,false]
      var expected=cells.map(function(cell){return cell.modelData.have})
      if(expected.some(function(have,index){return have!==wanted[index]}) && root.waits++<60)return
      root.check("priority preserves the same verified/missing colours as ordinary pieces",{colors:colors,have:expected},colors[0]===colors[root.phase===0 ? 2 : 3] && colors[1]===colors[root.phase===0 ? 3 : 2] && colors[0]!==colors[1])
      root.check("priority is a secondary mark below completion for both verified and missing cells",marks,marks[0]==="▲" && marks[1]==="▲" && marks[2]==="" && marks[3]==="" && root.part(cells[0],"piece-priority").y>=root.part(cells[0],"piece-completion").height)
      if(root.phase++===0){root.waits=0;bar.snapshot={count:4,piece_size:16384,have:"YA==",priority:[0,1],playable_end:0};return}
      console.log("FBTQS-PASS priority_completion");stop();Quickshell.execDetached(["qs","kill","-p",Quickshell.shellPath(".")])
    }
  }
}
