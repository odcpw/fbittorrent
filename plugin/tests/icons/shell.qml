import QtQuick
import QtQuick.Window
import Quickshell
import Quickshell.Io
import qs.Commons as Commons
import "Plugin/components"
import "Plugin/Store.js" as Store
import "Plugin/Glyphs.js" as Glyphs

ShellRoot {
  id: root
  property var profiles: []
  property var palettes: []
  property int cursor: 0
  FileView { id:input; blockLoading:true; blockAllReads:true; preload:false }
  QtObject {
    id: service
    property bool online:false
    property bool reconnecting:false
    property bool stoppedRecently:false
    property bool startupRequested:false
    property string byeReason:""
    property string backendState:"running"
    property string lastError:""
    property var status:({})
    property var store:Store.create()
    property var client:({caps:[]})
  }
  QtObject {
    id: bar
    property color barForeground:"white"
    property color foreground:barForeground
    property color background:"black"
    property color urgent:"red"
    property bool foregroundAnimationEnabled:false
    property bool vertical:false
    property string fontFamily:Quickshell.env("FBT_FONT_FAMILY")
    property int barSize:32
    function hideTooltip(item) {}
    function registerClickTarget(item) {}
    function unregisterClickTarget(item) {}
  }
  Window {
    visible:true
    width:480; height:84
    color:bar.background
    Item {
      id: scene
      anchors.fill:parent
      Rectangle { anchors.fill:parent; color:bar.background }
      BarIndicator { id:indicator; service:service; bar:bar; showRates:true; x:14; y:12; height:32; width:implicitWidth }
      Text {
        x:14; y:50; width:parent.width-28
        textFormat:Text.PlainText
        text:indicator.tooltipText
        color:bar.barForeground
        font.family:bar.fontFamily; font.pixelSize:Commons.Style.font.bodySmall
        elide:Text.ElideRight
      }
    }
  }
  function next() {
    if(cursor>=profiles.length*palettes.length) {
      console.log("FBTQS-PASS bar_icons")
      Qt.callLater(function(){Quickshell.execDetached(["qs","kill","-p",Quickshell.shellPath(".")])})
      return
    }
    var profile=profiles[cursor%profiles.length], palette=palettes[Math.floor(cursor/profiles.length)]
    bar.barForeground=palette.foreground;bar.background=palette.background;bar.urgent=palette.red
    service.online=profile.online;service.reconnecting=!!profile.reconnecting;service.stoppedRecently=!!profile.stoppedRecently
    service.startupRequested=!!profile.startupRequested;service.byeReason=profile.byeReason || ""
    service.backendState=profile.backendState || "running";service.store=Store.create()
    service.store.apply({id:1,ok:true,result:{torrents:profile.rows || []}},{op:"torrents.list"})
    service.store.summary=profile.summary
    service.client={caps:profile.caps || []};service.status={dl:profile.summary.dl_bps,up:profile.summary.ul_bps}
    delay.restart()
  }
  Timer {
    id:delay; interval:60
    onTriggered: {
      var profile=root.profiles[root.cursor%root.profiles.length], palette=root.palettes[Math.floor(root.cursor/root.profiles.length)]
      var appearance=indicator.appearance
      var ok=appearance.state===profile.expectedState && appearance.tone===profile.expectedTone && appearance.glyph===Glyphs.glyph(profile.expectedGlyph)
      ok=ok && indicator.tooltipText.indexOf(profile.expectedText)>=0 && indicator.tooltipText.indexOf("<")<0
      var path=Quickshell.env("FBT_CAPTURE_DIR")+"/bar-"+profile.name+"-"+palette.name+".png"
      scene.grabToImage(function(result){
        var saved=result.saveToFile(path)
        console.log("FBTQS-STEP "+JSON.stringify({case:"bar_icons",action:"capture "+profile.name+" "+palette.name,
          expect:"correct glyph, weight and words; PNG saved",observed:JSON.stringify(appearance)+"; saved="+saved,ok:ok && saved,ms:0,artifacts:[path]}))
        root.cursor++;root.next()
      })
    }
  }
  Component.onCompleted: {
    input.path=Quickshell.shellPath("Icons.json");profiles=JSON.parse(input.text())
    input.path=Quickshell.shellPath("Palettes.json");palettes=JSON.parse(input.text())
    next()
  }
}
