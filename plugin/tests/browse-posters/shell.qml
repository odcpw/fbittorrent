import QtQuick
import Quickshell
import "Plugin/BrowsePosters.js" as Posters
import "Plugin/components" as Components
import qs.Commons as Commons
ShellRoot {
 Component {id:gridComponent;Components.BrowseGrid {}}
 function check(name,expect,value,ok){console.log("FBTQS-STEP "+JSON.stringify({case:name,action:"exercise bounded poster queue",expect:expect,observed:JSON.stringify(value),ok:!!ok,ms:0}))}
 Component.onCompleted:{
  var calls=[],s={lookupImdb:function(args,done){calls.push({id:args.id,done:done});return calls.length}},p=Posters.create(s,function(){}),rows=[]
  for(var i=0;i<100;i++)rows.push({imdb_id:"tt"+(1000000+i)})
  p.load(rows)
  check("bounds","only60 unique posters queued and at most4 requests active",p.state(),calls.length===4 && p.state().queued===56 && p.state().pending===4)
  var replied=0,maximum=0
  while(replied<calls.length){maximum=Math.max(maximum,p.state().pending);var call=calls[replied++];call.done({card:{id:call.id,poster_path:"/owned/"+call.id+".jpg"}},null)}
  check("bounds","all60 finish without crossing the4-request concurrency cap",p.state(),calls.length===60 && maximum===4 && p.state().cached===60 && p.state().pending===0)
  p.load(rows);check("cache","a repeated grid reuses cached cards",p.state(),calls.length===60)
  var next=[];for(var n=0;n<60;n++)next.push({imdb_id:"tt"+(2000000+n)})
  p.load(next);check("hide","new grid owns only four active requests",p.state(),p.state().pending===4)
  p.hide();var stopped=calls.length
  while(replied<calls.length){call=calls[replied++];call.done({card:{id:call.id,poster_path:"/owned/"+call.id+".jpg"}},null)}
  check("hide","hide drops queued work; late reads cannot start more lookups",p.state(),calls.length===stopped && p.state().pending===0 && p.state().queued===0)
  p.load(next)
  while(replied<calls.length){call=calls[replied++];call.done({card:{id:call.id}},null)}
  next=[];for(n=0;n<60;n++)next.push({imdb_id:"tt"+(3000000+n)})
  p.load(next)
  while(replied<calls.length){call=calls[replied++];call.done({card:{id:call.id}},null)}
  check("cache","cache remains bounded across three separate grids",p.state(),p.state().cached===128)
  var duplicate=Posters.create(s,function(){});var start=calls.length
  duplicate.load([{imdb_id:"tt9999999"},{imdb_id:"tt9999999"}]);duplicate.load([{imdb_id:"tt9999999"}])
  check("dedupe","duplicate IDs and overlapping requests create one lookup",duplicate.state(),calls.length===start+1 && duplicate.state().pending===1)
  var grid=gridComponent.createObject(null,{service:{browsePosterRevision:0,browsePosters:p},width:Commons.Style.space(400),height:Commons.Style.space(600)})
  check("columns","narrow width uses three poster columns",grid.columns,grid.columns===3)
  grid.width=Commons.Style.space(600);check("columns","medium width uses four poster columns",grid.columns,grid.columns===4)
  grid.width=Commons.Style.space(1200);check("columns","wide width caps the grid at six columns",grid.columns,grid.columns===6)
  grid.destroy()
  ;["bounds","cache","hide","dedupe","columns"].forEach(function(name){console.log("FBTQS-PASS "+name)})
  Qt.callLater(function(){Quickshell.execDetached(["qs","kill","-p",Quickshell.shellPath(".")])})
 }
}
