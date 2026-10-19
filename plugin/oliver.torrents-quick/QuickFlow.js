.pragma library

// One bounded search consumer on the existing service socket, never a helper.
function create(service,changed) {
  var generation=0
  var f={opened:false,query:"",session:0,rows:[],cursor:0,phase:"idle",message:""}
  function publish(){changed()}
  function stop(){
    if(f.session){if(service.online)service.mutate("search.stop",{search_id:f.session},"search");service.store.releaseTopics(["search:"+f.session])}
    f.session=0;if(service.quickTopics.length)service.quickTopics=[]
  }
  f.open=function(){generation++;stop();f.opened=true;f.query="";f.rows=[];f.cursor=0;f.phase="idle";f.message="";service.quickVisible=true;publish()}
  f.close=function(){generation++;stop();f.opened=false;f.rows=[];service.quickVisible=false;publish()}
  f.error=function(message){f.phase="error";f.message=String(message).slice(0,1024);publish()}
  f.source=function(text){var parsed=service.parseAddSource(text);return parsed && (parsed.source.magnet || parsed.source.path) ? parsed.source : null}
  f.looksSource=function(text){return /^(?:magnet:|https?:|file:\/\/|\/)/i.test(String(text).trim())}
  f.edited=function(text){if(f.phase!=="adding" && f.query!==text){generation++;stop();f.rows=[];f.cursor=0;f.phase="idle";f.message="";publish()}}
  f.search=function(text){
    var query=String(text).trim();if(!query)return false
    if(f.looksSource(query)){if(!f.source(query))f.error("Enter a valid magnet or absolute .torrent path");return false}
    if(query.length>256){f.error("Search is limited to 256 characters");return false}
    if(query===f.query && f.session)return true
    stop();var version=++generation;f.query=query;f.rows=[];f.cursor=0;f.phase="searching";f.message="";publish()
    var sent=service.mutate("search.start",{query:query,limit:10},"search",function(result,failure){
      if(version!==generation || !f.opened){if(result)service.mutate("search.stop",{search_id:result.search_id},"search");return}
      if(failure){f.error(failure.message);return}
      f.session=result.search_id;service.quickTopics=["search:"+f.session];publish()
    });return !!sent
  }
  f.move=function(delta){f.cursor=Math.max(0,Math.min(f.rows.length-1,f.cursor+delta));publish()}
  f.current=function(){return f.rows[f.cursor] || null}
  f.add=function(source,done){
    if(f.phase==="adding")return false
    f.phase="adding";f.message="";publish()
    var sent=service.mutate("torrents.add",{source:source},"torrents",function(result,failure){
      if(failure){if(f.opened)f.error(failure.message);else service.notify(failure.message);return}
      // An accepted unresolved magnet is already a durable record; metadata
      // completion is observed by the shared service after this overlay closes.
      service.notify(result.duplicate ? "Already in your list" : "Torrent added");if(f.opened)done()
    });return !!sent
  }
  f.frame=function(frame){
    if(!f.opened)return
    if(frame.topic==="search:"+f.session && ["snapshot.end","delta"].indexOf(frame.ev)>=0){
      var keyed=service.store.topics[frame.topic] || {};f.rows=Object.keys(keyed).slice(0,10).map(function(k){return keyed[k]});f.cursor=Math.min(f.cursor,Math.max(0,f.rows.length-1));publish()
    }
    function event(e){if(e.ev==="search.done" && e.search_id===f.session){f.phase="done";f.message=(e.warnings || []).map(function(w){return w.provider+": "+w.message}).join("; ");publish()}}
    event(frame)
    if(frame.topic==="events")(frame.items || frame.added || []).forEach(event)
  }
  f.snapshot=function(){return {opened:f.opened,query:f.query,session:f.session,rows:f.rows,cursor:f.cursor,phase:f.phase,message:f.message}}
  return f
}
