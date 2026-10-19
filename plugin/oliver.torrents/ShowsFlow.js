.pragma library
function create(service,changed){
  var f={selected:"inbox",focus:"shows",cursor:0,panel:"",error:"",busy:false,shows:[],rows:[]},active=false,reference="",completion=null
  function topic(){return "shows.releases:"+f.selected}
  function sync(){
    var selected=f.current(),key=selected ? selected.show_imdb+"/"+selected.key : ""
    var shows=service.store.topics.shows || {},releases=service.store.topics[topic()] || {}
    f.shows=Object.keys(shows).map(function(k){return shows[k]}).sort(function(a,b){return a.title.localeCompare(b.title) || a.imdb.localeCompare(b.imdb)})
    f.rows=Object.keys(releases).map(function(k){return releases[k]}).sort(function(a,b){return (b.published_at || 0)-(a.published_at || 0) || a.key.localeCompare(b.key) || a.show_imdb.localeCompare(b.show_imdb)})
    var index=f.rows.findIndex(function(r){return r.show_imdb+"/"+r.key===key})
    f.cursor=Math.max(0,Math.min(f.rows.length-1,index<0 ? f.cursor : index));changed()
  }
  function finish(result,failure){
    if(result && result.state==="pending")return
    var done=completion;completion=null;reference="";f.busy=false
    if(failure)f.error=failure.message || "The request failed"
    else if(done)done(result || {})
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
  f.mount=function(){active=true;service.showsTopics=[topic()];sync()}
  f.hide=function(){active=false;service.showsTopics=[];delete service.store.topics[topic()];f.rows=[]}
  f.select=function(id,child){var previous=topic();f.selected=id;if(previous!==topic())delete service.store.topics[previous];f.cursor=0;if(child)f.focus="releases";if(active)service.showsTopics=[topic()];sync()}
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
    if((frame.topic==="shows" || frame.topic===topic()) && ["snapshot.end","delta"].indexOf(frame.ev)>=0 || request && request.op==="shows.list")sync()
  }
  f.snapshot=function(){return {selected:f.selected,focus:f.focus,cursor:f.cursor,shows:f.shows,rows:f.rows,panel:f.panel,busy:f.busy,error:f.error}}
  return f
}
