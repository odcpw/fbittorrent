.pragma library
var qualities=["2160p","1080p","720p","480p"]
function create(service,changed){
  var f={selected:"inbox",focus:"shows",cursor:0,panel:"",error:"",busy:false,shows:[],rows:[],readFilter:"unread",qualityFilter:""},active=false,reference="",completion=null
  var collection=null,serial=0,readPending=false,readAgain=false
  function topic(){return "shows.releases:"+f.selected}
  function sync(){
    var selected=f.current(),key=selected ? selected.show_imdb+"/"+selected.key : ""
    var shows=service.store.topics.shows || {},releases=service.store.topics[topic()] || {}
    f.shows=Object.keys(shows).map(function(k){return shows[k]}).sort(function(a,b){return a.title.localeCompare(b.title) || a.imdb.localeCompare(b.imdb)})
    f.rows=(collection || Object.keys(releases).map(function(k){return releases[k]})).filter(function(row){return (f.readFilter==="all" || row.read===(f.readFilter==="read")) && (!f.qualityFilter || row.quality===f.qualityFilter)})
    f.rows.sort(function(a,b){return (b.published_at || 0)-(a.published_at || 0) || a.key.localeCompare(b.key) || a.show_imdb.localeCompare(b.show_imdb)})
    var index=f.rows.findIndex(function(r){return r.show_imdb+"/"+r.key===key})
    f.cursor=Math.max(0,Math.min(f.rows.length-1,index<0 ? f.cursor : index));changed()
  }
  // The pushed inbox contains unread rows. One bounded read also supplies read
  // history for All/Read filters; pushed changes invalidate this visible cache.
  function fetch(){
    readAgain=true
    if(readPending || !active)return
    readAgain=false;readPending=true;changed()
    var generation=serial,args={unread_only:false}
    if(f.selected==="inbox")args.inbox=true;else args.imdb=f.selected
    service.client.request("shows.releases",args,{callback:function(result,failure){
      readPending=false
      if(active && generation===serial){
        if(result && Array.isArray(result.releases) && result.releases.length<=100000){collection=result.releases;sync()}
        else if(failure){f.error=failure.message;changed()}
        else{f.error="The release list is too large to display";changed()}
      }
      if(readAgain && active)fetch()
    }})
  }
  function finish(result,failure){
    if(result && result.state==="pending")return
    var done=completion;completion=null;reference="";f.busy=false
    if(failure)f.error=failure.message || "The request failed"
    else {if(done)done(result || {});fetch()}
    changed()
  }
  function send(op,args,done){
    if(f.busy)return false
    f.error="";f.busy=true;completion=done
    var ref=service.mutate(op,args,"shows",finish)
    if(!ref){f.busy=false;completion=null;f.error=service.lastError || "Request was not sent"}
    else if(f.busy)reference=ref
    changed();return !!ref
  }
  f.current=function(){return f.rows[f.cursor] || null}
  f.preferenceTarget=function(){var row=f.current(),id=f.selected!=="inbox" ? f.selected : row ? row.show_imdb : "";return f.shows.find(function(s){return s.imdb===id}) || null}
  f.setQuality=function(value){var show=f.preferenceTarget();if(show && (value==="" || qualities.indexOf(value)>=0))send("shows.set_quality",{imdb:show.imdb,quality:value || null})}
  f.filters=function(read,quality){if(["unread","all","read"].indexOf(read)<0 || quality && qualities.indexOf(quality)<0)return;f.readFilter=read;f.qualityFilter=quality;f.cursor=0;sync();if(!collection)fetch()}
  f.mount=function(){active=true;serial++;service.showsTopics=[topic()];sync();fetch()}
  f.hide=function(){active=false;serial++;service.showsTopics=[];delete service.store.topics[topic()];collection=null;f.rows=[]}
  f.select=function(id,child){
    if(id===f.selected){if(child)f.focus="releases";changed();return}
    var previous=topic();f.selected=id;serial++;collection=null;delete service.store.topics[previous]
    f.readFilter=id==="inbox" ? "unread" : "all";f.cursor=0;if(child)f.focus="releases";if(active)service.showsTopics=[topic()];sync();fetch()
  }
  f.move=function(delta,edge){
    if(f.focus==="shows"){
      var ids=["inbox"].concat(f.shows.map(function(s){return s.imdb})),index=ids.indexOf(f.selected)
      f.select(ids[Math.max(0,Math.min(ids.length-1,edge===undefined ? index+delta : edge))],false)
    }else{f.cursor=Math.max(0,Math.min(f.rows.length-1,edge===undefined ? f.cursor+delta : edge));changed()}
  }
  f.pane=function(child){f.focus=child ? "releases" : "shows";changed()}
  f.primary=function(){if(f.focus==="shows"){f.pane(true);return}var row=f.current();if(row)service.addFlow.release("shows.grab",{imdb:row.show_imdb,key:row.key},row)}
  f.followOpen=function(){f.error="";f.panel="follow";changed()}
  f.follow=function(query){query=String(query).trim();if(!query)return;send("shows.follow",/^tt\d+$/.test(query) ? {imdb:query} : {query:query},function(){f.panel=""})}
  f.cancel=function(){if(f.busy)return;f.panel="";f.error="";changed()}
  f.mark=function(){var row=f.current();if(row)send("shows.set_read",{imdb:row.show_imdb,key:row.key,read:!row.read})}
  f.markAll=function(){
    var ids=f.selected==="inbox" ? f.shows.map(function(s){return s.imdb}) : [f.selected]
    function next(){if(ids.length)send("shows.mark_show_read",{imdb:ids.shift()},next)}next()
  }
  f.undo=function(){send("shows.undo_read",{})}
  f.check=function(selected){var row=f.current(),id=f.selected!=="inbox" ? f.selected : row ? row.show_imdb : "";send("shows.check",selected && id ? {imdb:id} : {})}
  f.removeOpen=function(){var row=f.current();f.removing=f.selected!=="inbox" ? f.selected : row ? row.show_imdb : "";if(f.removing){f.panel="remove";changed()}}
  f.remove=function(){if(f.removing)send("shows.unfollow",{imdb:f.removing},function(){f.panel="";f.select("inbox",false)})}
  f.frame=function(frame,request){
    if(reference){var outcome=service.store.mutations[reference];if(outcome && outcome.status!=="pending")finish(outcome.status==="done" ? outcome.result || {} : null,outcome.error || (outcome.status==="done" ? null : {message:"Result unknown; refresh to check"}))}
    if(!active)return
    if((frame.topic==="shows" || frame.topic===topic()) && ["snapshot.end","delta"].indexOf(frame.ev)>=0 || request && request.op==="shows.list"){sync();fetch()}
  }
  f.snapshot=function(){var show=f.preferenceTarget();return {selected:f.selected,focus:f.focus,cursor:f.cursor,shows:f.shows,rows:f.rows,panel:f.panel,busy:f.busy,error:f.error,readFilter:f.readFilter,qualityFilter:f.qualityFilter,preferenceTarget:show ? show.imdb : "",preference:show && show.quality || "",preferenceTitle:show ? show.title : "",loading:readPending}}
  return f
}
