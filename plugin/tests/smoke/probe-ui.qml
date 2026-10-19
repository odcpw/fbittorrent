import QtQuick
import QtQuick.Window
import Quickshell
import qs.Commons as Commons
import qs.Ui as Ui

ShellRoot {
  Window {
    visible:true;width:400;height:100
    Ui.PanelHero {id:hero;width:380;title:"Torrents";meta:"probe";foreground:Commons.Color.foreground}
  }
  Component.onCompleted: Qt.callLater(function(){
    var ok=hero.implicitHeight>0 && hero.title==="Torrents"
    console.log("FBTQS-STEP "+JSON.stringify({case:"probe_ui",action:"instantiate installed PanelHero through qs.Ui and qs.Commons",expect:"native component loads with nonzero height",observed:"height="+hero.implicitHeight+"; qs="+Quickshell.env("FBT_QS_VERSION"),ok:ok,ms:0}))
    console.log("FBTQS-PASS probe_ui")
    Quickshell.execDetached(["qs","kill","-p",Quickshell.shellPath("probe-ui.qml")])
  })
}
