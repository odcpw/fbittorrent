import QtQuick
import Quickshell
import Quickshell.Io
import "Plugin/Model.js" as Model
import "Plugin/Palette.js" as Palette
import "Plugin/DetailText.js" as DetailText

ShellRoot {
  FileView {id:input;blockLoading:true;blockAllReads:true;preload:false}
  function check(name,action,expect,observed,ok) {
    console.log("FBTQS-STEP "+JSON.stringify({case:name,action:action,expect:expect,observed:String(observed),ok:!!ok,ms:0}))
  }
  function verdict(name) {console.log("FBTQS-PASS "+name)}
  Component.onCompleted: {
    input.path=Quickshell.shellPath("FormatCases.json")
    var cases=JSON.parse(input.text())
    check("format","load literal cases","at least 40 boundary cases",cases.length,cases.length>=40)
    cases.forEach(function(test){var actual=Model[test.fn].apply(null,test.args);check("format",test.fn+JSON.stringify(test.args),JSON.stringify(test.expect),actual,actual===test.expect)})
    check("format","downloading rows do not describe upload caps as seeding","plain progress and explicit Details policy",Model.meta({state:"downloading",seed:false,progress:0.5}),Model.meta({state:"downloading",seed:false,progress:0.5})==="50%" && DetailText.policy({seed:true,upload_enabled:false})==="Upload: off" && DetailText.policy({seed:false,upload_enabled:true})==="Upload: on")
    verdict("format")
    input.path=Quickshell.shellPath("ThemeExpected.json")
    var themes=JSON.parse(input.text()),names=Object.keys(themes)
    check("dim_ladder","load theme oracle","installed theme copies plus owner",names.join(","),names.length>=20 && names.indexOf("owner")>=0)
    names.forEach(function(name){
      var t=themes[name],actual=Model.dim(t.foreground,t.background),a=Palette.channels(actual),b=Palette.channels(t.dim)
      check("dim_ladder",name+" dim",t.dim+" within one per channel",actual,a.every(function(v,i){return Math.abs(v-b[i])<=1}) && Palette.contrast(actual,t.background)>=4.5)
      var accent=Model.accentText(t.accent,t.background,t.foreground)
      check("accent_fallback",name+" accent",t.accent_text ? "accent, regular" : "foreground, bold",JSON.stringify(accent),String(accent.color)===(t.accent_text ? t.accent : t.foreground) && accent.bold===!t.accent_text)
    })
    verdict("dim_ladder");verdict("accent_fallback")
    Qt.callLater(function(){Quickshell.execDetached(["qs","kill","-p",Quickshell.shellPath(".")])})
  }
}
