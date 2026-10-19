import QtQuick
import Quickshell
import "Plugin/ShowsFlow.js" as Shows
ShellRoot {
  function require(value,message){if(!value)throw Error(message)}
  Component.onCompleted:{
    try {
      var reads=[],actions=[],service={showsTopics:[],store:{topics:{shows:{tt1:{imdb:"tt1",title:"One",quality:"2160p",unread:1}}},mutations:{}}}
      service.client={caps:["shows"],request:function(op,args,options){reads.push({op:op,args:args,done:options.callback});return reads.length}}
      service.mutate=function(op,args,cap,done){actions.push({op:op,args:args,done:done});return "ref-"+actions.length}
      var flow=Shows.create(service,function(){})
      var rows=[{show_imdb:"tt1",key:"S01E01",quality:"2160p",read:false},{show_imdb:"tt1",key:"S01E02",quality:"720p",read:true},{show_imdb:"tt1",key:"S01E03",quality:"1080p",read:false}]
      flow.mount();require(reads.length===1 && reads[0].op==="shows.releases" && reads[0].args.unread_only===false,"read history is explicitly fetched without a download")
      reads[0].done({releases:rows},null);require(flow.rows.length===2,"inbox defaults to unread")
      flow.filters("all","");require(flow.rows.length===3,"All includes confirmed read rows")
      flow.filters("read","720p");require(flow.rows.length===1 && flow.rows[0].key==="S01E02","read and quality filters intersect")
      flow.filters("unread","720p");require(!flow.rows.length,"quality filter never invents an unread row")
      flow.filters("all","2160p");require(flow.rows.length===1 && flow.rows[0].key==="S01E01","All and quality remain independent")
      flow.setQuality("720p");require(actions.length===1 && actions[0].op==="shows.set_quality" && actions[0].args.quality==="720p","preference writes the selected show's daemon intent")
      require(flow.snapshot().preference==="2160p","pending preference never changes confirmed state")
      actions[0].done({show:{imdb:"tt1",quality:"720p"}},null)
      flow.setQuality("");require(actions.length===2 && actions[1].args.quality===null,"Default explicitly clears the override")
      actions[1].done({},null);flow.setQuality("4k");require(actions.length===2,"invalid choices never send")
      // A slow read cannot install the previous show's collection or fan out.
      var pending=reads[reads.length-1];flow.select("tt2",false);flow.select("tt1",false)
      require(reads.length===2,"rapid selections coalesce behind the one outstanding read")
      pending.done({releases:[{show_imdb:"tt2",key:"wrong",quality:"2160p",read:false}]},null)
      require(reads.length===3 && reads[2].args.imdb==="tt1" && !flow.rows.some(function(r){return r.key==="wrong"}),"stale read is ignored and latest selection is fetched")
      reads[2].done({releases:rows},null);flow.hide();require(!service.showsTopics.length && !flow.rows.length,"hiding releases observation and the history cache")
      require(actions.every(function(a){return a.op==="shows.set_quality"}),"filters/preferences never grab a torrent")
      console.log("FBTQS-PASS quality_and_filters")
    }catch(error){console.log("FBTQS-FAIL quality_and_filters "+String(error))}
    Qt.callLater(function(){Quickshell.execDetached(["qs","kill","-p",Quickshell.shellPath(".")])})
  }
}
