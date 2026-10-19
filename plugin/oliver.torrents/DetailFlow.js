.pragma library

// One controller belongs to the shared service, not either layout Loader.
function create(service, changed) {
  var settingsInstance="", engineEpoch=0, metadataKnown=false
  var pieceCache=null, pieceMap=null, pieceState="",cachedRow=null,cachedVersion=-1,seenVersion=-1,fileVersion=-1
  var extras=Object.create(null), looked=Object.create(null), cards=Object.create(null)
  var d={hash:"",tab:"overview",query:"",files:[],cursor:0,card:null,loading:false,trackerBusy:false,trackerRequest:0}
  function keep(cache,key,value){if(!Object.prototype.hasOwnProperty.call(cache,key) && Object.keys(cache).length>=128)delete cache[Object.keys(cache)[0]];cache[key]=value}
  function cap(name){return !!service.client && service.client.caps.indexOf(name)>=0}
  d.tabs=function(){var tabs=["overview","files"];["peers","trackers","pieces"].forEach(function(c){if(cap(c))tabs.push(c)});tabs.push("log");return tabs}
  d.row=function(){var version=service.store.rowVersions[d.hash] || 0;if(cachedVersion!==version){cachedVersion=version;cachedRow=null}var live=service.store.torrents[d.hash];if(live && !cachedRow)cachedRow=Object.assign({},extras[d.hash] || {},live);return live ? cachedRow : null}
  d.get=function(force){
    var hash=d.hash
    if(!hash || (!force && extras[hash]))return
    service.client.request("torrents.get",{hash:hash},{callback:function(result){
      if(!result)return
      var extra={},row=result.torrent
      ;["original_name","file_count","piece_count","piece_size","tracker_count","private","undo_until","comment","created_by"].forEach(function(k){if(row[k]!==undefined)extra[k]=row[k]})
      keep(extras,hash,extra);if(d.hash===hash){cachedRow=null;changed()}
    }})
  }
  d.lookup=function(){
    var row=d.row(),hash=d.hash
    if(!row || !cap("imdb"))return
    var key=row.imdb_id || hash
    if(cards[key]){d.card=cards[key];changed();return}
    if(Object.prototype.hasOwnProperty.call(looked,key)){
      if(looked[key] && !cards[looked[key]])delete looked[key]
      else {d.card=looked[key] ? cards[looked[key]] : null;changed();return}
    }
    // Cache nulls too. Mark before dispatch so two opens cannot race a lookup.
    keep(looked,key,null)
    service.lookupImdb(row.imdb_id ? {id:row.imdb_id} : {name:row.name},function(result,failure){
      if(failure){delete looked[key];return}
      var card=result ? result.card : null
      if(card){keep(cards,card.id,card);keep(looked,key,card.id)}
      if(d.hash===hash){d.card=card;changed()}
    })
  }
  d.begin=function(hash){
    if(hash===d.hash)return
    pieceCache=null;pieceMap=null;pieceState="";cachedRow=null;cachedVersion=-1;seenVersion=service.store.rowVersions[hash] || 0;fileVersion=-1
    service.unsubscribeDetail();service.logConsumer("details",false)
    d.trackerRequest++;d.trackerBusy=false;d.hash=hash || "";d.card=null;d.files=[];d.query="";d.cursor=0;d.loading=false
    if(d.hash){
      var live=service.store.torrents[d.hash];metadataKnown=!!(live && live.size!==undefined)
      engineEpoch=Number(service.store.info.engine_epoch || 0)
      if(cap("settings") && settingsInstance!==service.instance){settingsInstance=service.instance;service.client.request("app.settings.get",{})}
      d.get(false);d.topics()
    }
    changed()
  }
  d.hide=function(){d.begin("")}
  d.topics=function(){
    service.logConsumer("details",!!d.hash && d.tab==="log")
    var topic=d.tab==="files" ? "files" : ["peers","trackers","pieces"].indexOf(d.tab)>=0 ? d.tab : ""
    if(d.hash && topic){
      if(topic==="files")fileVersion=-1
      var row=d.row()
      if(topic==="pieces" && pieceCache && row && row.checked===true && row.state===pieceState)service.store.setPieces("torrent:"+d.hash+".pieces",pieceCache,pieceMap)
      service.subscribeDetail(d.hash,[topic]);d.loading=topic==="files" && !service.store.topics["torrent:"+d.hash+".files"]

    }else service.unsubscribeDetail()
    d.refreshFiles()
  }
  d.reannounce=function(){
    if(!d.hash || d.tab!=="trackers")return false
    if(d.trackerBusy)return true
    var hash=d.hash,request=++d.trackerRequest;d.trackerBusy=true;changed()
    var sent=service.mutate("torrents.reannounce",{hashes:[hash]},"trackers",function(){if(d.hash===hash && d.trackerRequest===request){d.trackerBusy=false;changed()}})
    if(!sent){d.trackerBusy=false;changed()}
    return !!sent
  }
  d.editTracker=function(url,add,done){
    if(!d.hash || d.tab!=="trackers" || d.trackerBusy)return false
    var hash=d.hash,request=++d.trackerRequest;d.trackerBusy=true;changed()
    var sent=service.mutate(add ? "torrents.add_tracker" : "torrents.remove_tracker",{hash:hash,url:url},"tracker_edit",function(result,failure){
      if(d.hash===hash && d.trackerRequest===request){d.trackerBusy=false;changed();if(done)done(result,failure)}
    })
    if(!sent){d.trackerBusy=false;changed()}
    return !!sent
  }
  d.selectTab=function(tab){if(d.tabs().indexOf(tab)<0)return;d.tab=tab;d.query="";d.cursor=0;if(d.hash)d.topics();changed()}
  d.cycle=function(delta){var tabs=d.tabs();d.selectTab(tabs[(tabs.indexOf(d.tab)+delta+tabs.length)%tabs.length])}
  d.refreshFiles=function(){
    var keyed=service.store.topics["torrent:"+d.hash+".files"] || {},query=d.query.toLocaleLowerCase()
    d.files=Object.keys(keyed).map(function(k){return keyed[k]}).filter(function(f){return !query || f.path.toLocaleLowerCase().indexOf(query)>=0})
    d.files.sort(function(a,b){return a.sort_key<b.sort_key ? -1 : a.sort_key>b.sort_key ? 1 : a.index-b.index})
    d.cursor=Math.max(0,Math.min(d.files.length-1,d.cursor));changed()
  }
  d.filter=function(text){d.query=String(text).slice(0,200);d.cursor=0;d.refreshFiles()}
  d.move=function(delta){d.cursor=Math.max(0,Math.min(d.files.length-1,d.cursor+delta));changed()}
  d.setFiles=function(included,all){
    var row=d.row();if(!row || row.state==="checking")return false
    var selected=all ? d.files.map(function(f){return f.index}) : d.files[d.cursor] ? [d.files[d.cursor].index] : []
    return service.setFiles(d.hash,selected,included)
  }
  d.toggle=function(){var f=d.files[d.cursor];return f ? d.setFiles(!f.included,false) : false}
  d.openFile=function(){var f=d.files[d.cursor];if(f)service.openTarget(d.hash,f.index)}
  d.frame=function(frame,request){
    if(!d.hash)return
    if(frame.topic==="torrent:"+d.hash+".pieces"){
      var current=service.store.torrents[d.hash]
      if(current && current.checked===true && current.state!=="checking" && current.state!=="error"){
        pieceCache=service.store.topics[frame.topic] || null;pieceMap=service.store.pieces[frame.topic] || null;pieceState=current.state
      }
      return
    }
    var topic="torrent:"+d.hash+".files"
    if(frame.topic===topic){
      if(frame.ev==="snapshot.end")d.loading=false
      if((service.store.topicVersions[topic] || 0)!==fileVersion && frame.ev!=="snapshot.chunk" && frame.ev!=="snapshot.begin"){fileVersion=service.store.topicVersions[topic] || 0;d.refreshFiles()}
    }
    var events=frame.topic==="events" ? frame.items || frame.added || [] : [frame]
    var engineChanged=events.some(function(e){if(e.ev!=="engine.changed" && e.ev!=="engine.restarted")return false;var epoch=Number(e.epoch || engineEpoch+1);if(epoch<=engineEpoch)return false;engineEpoch=epoch;return true})
    var row=service.store.torrents[d.hash],known=!!(row && row.size!==undefined)
    if(engineChanged || !row || row.checked!==true || row.state==="checking" || row.state==="error" || (pieceCache && row.state!==pieceState) || (request && request.ref && frame.ok)){
      pieceCache=null;pieceMap=null;pieceState="";service.store.releaseTopics(["torrent:"+d.hash+".pieces"])
      if(d.tab==="pieces")changed()
    }
    if(engineChanged || (known && !metadataKnown))d.get(true)
    var version=service.store.rowVersions[d.hash] || 0
    if(version!==seenVersion){seenVersion=version;changed()}
    metadataKnown=known
  }
  d.reset=function(){extras=Object.create(null);looked=Object.create(null);cards=Object.create(null);var hash=d.hash;d.hash="";d.begin(hash)}
  return d
}
