import QtQuick
import QtQuick.Window
import Quickshell
import "Plugin"
import "Plugin/views" as Views
ShellRoot {
 id:root
 property string secret:"synthetic-passkey"
 property string url:"https://feed.invalid/rss?passkey="+secret
 Service {id:backend}
 Window {visible:true;width:800;height:600
  Loader {id:rss;anchors.fill:parent;active:false;sourceComponent:Views.RssView {service:backend}}
 }
 function texts(item){var out=[];if(typeof item.text==="string")out.push(item.text);for(var i=0;i<item.children.length;i++)out=out.concat(texts(item.children[i]));return out}
 Component.onCompleted:Qt.callLater(function(){
  var h="a".repeat(40)
  backend.store.apply({id:1,ok:true,result:{torrents:[{hash:h,name:"Private fixture",state:"paused"}]}},{op:"torrents.list"});backend.detailFlow.hash=h
  backend.detailFlow.card={url:"javascript:alert(1)"};backend.titleLink(h,false)
  backend.detailFlow.card={url:"https://example.invalid/title"};backend.titleLink(h,false)
  backend.copyText("magnet:?xt=urn:btih:"+h+"&tr="+encodeURIComponent(url))
  var feed={id:"feed",name:"Private feed",url:url,error:"request failed: "+url+" denied "+secret,unread:0}
  backend.rssFlow={snapshot:function(){return {feeds:[feed],items:[],pane:"feeds",modal:"",message:"",feed:"feed"}},current:function(){return null},currentFeed:function(){return feed},mount:function(){},hide:function(){}}
  rss.active=true
 })
 Timer {interval:1000;running:true;onTriggered:{
  var text=root.texts(rss.item).join("\n"),ok=text.indexOf("request failed")>=0 && text.indexOf(root.secret)<0
  console.log("FBTQS-STEP "+JSON.stringify({case:"redaction",action:"render a feed and footer with echoed credentials",expect:"failure visible without the passkey",observed:ok ? "redacted" : "unsafe or missing text",ok:ok,ms:0}))
  console.log("FBTQS-PASS redaction");Quickshell.execDetached(["qs","kill","-p",Quickshell.shellPath(".")])
 }}
}
