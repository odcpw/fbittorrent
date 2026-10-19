.pragma library

function cardKey(row){return row.imdb_id || "name:"+row.name}
var SORTS=["seeds","size","uploaded","name","rating"]
function create(service,changed) {
  var browseRows=[]
  var epoch=0,cards=Object.create(null),installRef=""
  var s={query:"",label:"",mode:"query",session:0,visible:false,phase:"idle",rows:[],cursor:"",sort:"seeds",reverse:false,
    frames:0,total:0,card:null,panel:"",providers:[],sources:[],warnings:[],message:"",providerBusy:{}}
  function publish(){changed()}
  function cap(c){return !!service.client && service.client.caps.indexOf(c)>=0}
  function error(failure){s.phase="error";s.message=failure ? failure.message : "No answer";publish()}
  function stop(id){if(id){if(service.online)service.mutate("search.stop",{search_id:id},"search");service.store.releaseTopics(["search:"+id])}}
  function useRows(rows){
    s.total=rows.length
    var property={seeds:"seeds",size:"size",uploaded:"uploaded_at",name:"name",rating:"rating"}[s.sort]
    var descending=s.sort!=="name",direction=(descending!==s.reverse) ? -1 : 1
    var sorted=rows.slice().sort(function(a,b){
      var left=property==="rating" ? s.rating(a) : a[property],right=property==="rating" ? s.rating(b) : b[property],knownLeft=left!==undefined && left!==null,knownRight=right!==undefined && right!==null
      if(knownLeft!==knownRight)return knownLeft ? -1 : 1
      if(property==="name"){left=String(left).toLocaleLowerCase();right=String(right).toLocaleLowerCase()}
      if(left<right)return -direction;if(left>right)return direction
      return a.key<b.key ? -1 : a.key>b.key ? 1 : 0
    })
    s.rows=sorted.slice(0,500)
    if(!s.rows.some(function(r){return r.key===s.cursor}))s.cursor=s.rows.length ? s.rows[0].key : ""
    service.scheduleSearchPoster();publish()
  }
  s.rating=function(row){return row.rating!==undefined ? row.rating : cards[cardKey(row)] ? cards[cardKey(row)].rating : undefined}
  s.current=function(){for(var i=0;i<s.rows.length;i++)if(s.rows[i].key===s.cursor)return s.rows[i];return null}
  s.sync=function(){var keyed=service.store.topics["search:"+s.session] || {};useRows(Object.keys(keyed).map(function(k){return keyed[k]}))}
  s.mount=function(){s.visible=true;publish()}
  s.hide=function(){epoch++;s.visible=false;stop(s.session);service.searchTopics=[];s.session=0;s.panel="";publish()}
  s.cancel=function(){epoch++;stop(s.session);service.searchTopics=[];s.session=0;s.phase="stopped";publish()}
  s.start=function(query){
    query=String(query).trim().slice(0,256)
    if(!query)return false
    if(!cap("search")){error({message:"Search is unavailable; update the daemon"});return false}
    stop(s.session);service.searchTopics=[]
    var generation=++epoch;s.query=query;s.label=query;s.mode="query";s.session=0;s.phase="searching";s.rows=[];s.cursor="";s.card=null;s.message="";s.frames=0;s.warnings=[]
    service.mutate("search.start",{query:query,limit:5000},"search",function(result,failure){
      if(generation!==epoch || !s.visible){if(result)stop(result.search_id);return}
      if(failure){error(failure);return}
      s.session=result.search_id;service.searchTopics=["search:"+s.session];publish()
    });publish();return true
  }
  s.choose=function(key){if(s.cursor===key)return;s.cursor=key;s.card=null;service.scheduleSearchPoster();publish()}
  s.move=function(delta,absolute){var i=s.rows.findIndex(function(r){return r.key===s.cursor});i=absolute===undefined ? i+delta : absolute;i=Math.max(0,Math.min(s.rows.length-1,i));if(s.rows[i])s.choose(s.rows[i].key)}
  s.cycleSort=function(){s.sort=SORTS[(SORTS.indexOf(s.sort)+1)%SORTS.length];s.reverse=false;s.resort()}
  s.resort=function(){if(s.mode==="query" && s.session)s.sync();else useRows(browseRows)}
  s.reverseSort=function(){s.reverse=!s.reverse;s.resort()}
  s.lookup=function(){
    var row=s.current();if(!row || !cap("imdb") || !s.visible)return
    var id=cardKey(row)
    if(Object.prototype.hasOwnProperty.call(cards,id)){s.card=cards[id];publish();return}
    if(Object.keys(cards).length>=128)delete cards[Object.keys(cards)[0]]
    cards[id]=null
    service.lookupImdb(row.imdb_id ? {id:row.imdb_id} : {name:row.name},function(result,failure){
      if(failure){delete cards[id];return}
      cards[id]=result ? result.card : null
      if(s.visible && s.current() && cardKey(s.current())===id){s.card=cards[id];if(s.sort==="rating")s.resort();else publish()}
    })
  }
  s.activate=function(){
    var row=s.current();if(!row)return false
    if(row.added){service.finishAdd(row.added);return true}
    return service.beginSearchAdd(row,s.mode==="query" ? s.session : 0)
  }
  s.providersOpen=function(){s.panel="providers";s.providersRefresh();publish()}
  s.providersRefresh=function(){if(!cap("search"))return;service.client.request("search.providers",{}, {callback:function(result,failure){if(failure){s.message=failure.message}else s.providers=result.providers;publish()}})}
  s.providerToggle=function(row){
    if(s.providerBusy[row.name])return
    var pending=Object.assign({},s.providerBusy);pending[row.name]=true;s.providerBusy=pending;publish()
    service.mutate("search.set_provider",{name:row.name,enabled:!row.enabled},"search",function(result,failure){
      var next=Object.assign({},s.providerBusy);delete next[row.name];s.providerBusy=next
      if(failure)s.message=failure.message;else s.providersRefresh();publish()
    })
  }
  s.install=function(sources){
    s.message="Installing providers…";publish()
    var args=sources ? {sources:sources} : {curated:true}
    installRef=service.mutate("search.install",args,"search",function(result,failure){
      if(failure){s.message=failure.message;installRef="";publish()}
      else if(result.state!=="pending")installed(result)
    }) || ""
  }
  function installed(result){installRef="";s.message="Installed "+result.installed.length+" provider(s)"+(result.failed.length ? " · "+result.failed.map(function(f){return f.source+": "+f.message}).join("; ") : "");s.providersRefresh();publish()}
  s.browseOpen=function(){if(!cap("search")){error({message:"Browse is unavailable; update the daemon"});return}s.panel="browse";service.client.request("search.browse_sources",{}, {callback:function(result,failure){if(failure)s.message=failure.message;else s.sources=result.sources;publish()}});publish()}
  s.browse=function(source){
    if(!source.in_app){service.openUrl(source.url);return}
    stop(s.session);service.searchTopics=[];s.session=0
    var generation=++epoch;s.panel="";s.mode="browse";s.label=source.label;s.query=source.label;s.phase="searching";s.rows=[];s.card=null;s.message="";publish()
    service.client.request("search.browse",{source:source.id},{callback:function(result,failure){if(generation!==epoch || !s.visible)return;if(failure){error(failure);return}s.phase="done";browseRows=result.items.slice(0,5000);useRows(browseRows)}})
  }
  s.closePanel=function(){s.panel="";publish()}
  s.copy=function(){var row=s.current();if(!row)return;var magnet=row.magnet || (/^[0-9a-f]{40}$/.test(row.key) ? "magnet:?xt=urn:btih:"+row.key : "");if(magnet)service.copyText(magnet);else service.notify("This provider did not supply a magnet")}
  s.link=function(kind){var row=s.current();if(!row)return;var card=s.card,url=kind==="source" ? row.page_url || row.url || row.provider_url : kind==="wiki" ? card && card.wiki_url || "https://en.wikipedia.org/w/index.php?search="+encodeURIComponent(row.name) : card && card.url || (row.imdb_id ? "https://www.imdb.com/title/"+row.imdb_id+"/" : "https://www.imdb.com/find/?q="+encodeURIComponent(row.name));service.openUrl(url)}
  s.follow=function(){var row=s.current();var imdb=row ? row.imdb_id || (s.card ? s.card.id : "") : "";if(!imdb){service.notify("Choose a result with an IMDb identifier first");return false}return service.mutate("shows.follow",{imdb:imdb},"shows")}
  function event(e){
    if(e.ev==="search.done" && e.search_id===s.session){s.phase="done";s.warnings=e.warnings || [];s.message=s.warnings.map(function(w){return w.provider+": "+w.message}).join("; ");publish()}
    if(e.ev==="mutation.result" && e.ref===installRef){if(e.outcome==="done")installed(e.result);else {installRef="";s.message=e.error ? e.error.message : "Installation result unknown";publish()}}
  }
  s.frame=function(frame){
    if(frame.topic==="search:"+s.session){s.frames++;if(frame.ev==="snapshot.end" || frame.ev==="delta")s.sync()}
    if(frame.ev==="search.done" || frame.ev==="mutation.result")event(frame)
    if(frame.topic==="events"){
      if(frame.ev==="snapshot.chunk")(frame.items || []).forEach(event)
      if(frame.ev==="delta")(frame.added || []).forEach(event)
    }
  }
  s.snapshot=function(){return {query:s.query,label:s.label,mode:s.mode,session:s.session,phase:s.phase,rows:s.rows,cursor:s.cursor,sort:{key:s.sort,reverse:s.reverse},frames:s.frames,total:s.total,panel:s.panel,poster:s.card ? s.card.id : null,message:s.message}}
  return s
}
