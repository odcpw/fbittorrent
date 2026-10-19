.pragma library
.import "AddSource.js" as Source
.import "Model.js" as Model
function create(service,changed,debug) {
  var f={},epoch=0,trace=[],s={},readPending=false,filesRequested=false,pathDirty=false,spaceSerial=0,spaceDue=0,adopting=false
  function publish(){changed()}
  function record(kind,op,data){if(debug){trace.push({kind:kind,op:op,data:data});if(trace.length>32)trace.shift()}}
  function reset(){s={commitRequested:false,visible:false,phase:"closed",source:null,origin:"manual",hash:"",name:"",files:[],included:{},seed:false,savePath:"",error:"",duplicate:false,owned:false,stageSupported:false,created:service.now,elapsed_s:0,pendingRef:"",cancelRequested:false,space:{supported:false,pending:false}};readPending=false;filesRequested=false;pathDirty=false;spaceDue=0;adopting=false}
  reset()
  function requiredBytes(){
    if(s.files.length)return s.files.reduce(function(total,row){return total+(s.included[row.index] ? row.size : 0)},0)
    return s.size
  }
  function tooLarge(){var needed=requiredBytes();return needed!==undefined && s.space.free_bytes!==undefined && needed>s.space.free_bytes}
  function spaceReady(){return !s.space.supported || (!s.space.pending && s.space.writable===true && s.space.path===s.savePath && !tooLarge())}
  function checkSpace(){
    if(!s.visible || service.client.caps.indexOf("free_space")<0)return
    spaceDue=0;var path=s.savePath,generation=epoch,serial=++spaceSerial
    s.space={supported:true,path:path,pending:true}
    if(!path || path[0]!=="/"){s.space.pending=false;s.space.writable=false;s.space.message="Choose an absolute download folder";publish();return}
    service.client.request("app.free_space",{path:path},{callback:function(result,failure){
      if(generation!==epoch || serial!==spaceSerial || !s.visible)return
      s.space={supported:true,path:path,pending:false,writable:result ? result.writable : false}
      if(result && result.free_bytes!==undefined)s.space.free_bytes=result.free_bytes
      if(failure)s.space.message=failure.message
      else if(!s.space.writable)s.space.message="Folder is not writable"
      publish();if(s.commitRequested)f.commit()
    }})
    publish()
  }
  function scheduleSpace(immediate){
    if(service.client.caps.indexOf("free_space")<0)return
    spaceSerial++;s.space={supported:true,path:s.savePath,pending:true};spaceDue=service.now+250
    if(immediate)checkSpace()
  }
  function busy(){return ["staging","committing","discarding","cancel_wait","keeping"].indexOf(s.phase)>=0}
  function error(message){s.commitRequested=false;s.error=message || "No answer; refresh before trying again";s.phase="error";publish()}
  function clearTopics(){service.addTopics=[]}
  function close(){epoch++;s.visible=false;s.phase="closed";s.pendingRef="";s.commitRequested=false;clearTopics();publish()}
  function send(op,args,cap,done) {
    var generation=epoch
    record("request",op,args)
    var ref=service.mutate(op,args,cap,function(result,failure){
      if(generation!==epoch || !s.visible)return
      record("reply",op,result || failure)
      if(failure){error(failure.message);return}
      if(result.state==="pending")return
      done(result)
    })
    s.pendingRef=ref || "";s.pendingOp=op;s.pendingDone=done
    if(!ref)error(service.lastError || "Could not send the request")
  }
  function fileRows(rows) {
    if(["committing","keeping","discarding","cancel_wait"].indexOf(s.phase)>=0)return
    if(!Array.isArray(rows) || rows.length>8192){error("The file list is too large to display");return}
    s.files=rows.slice().sort(function(a,b){return a.sort_key<b.sort_key ? -1 : a.sort_key>b.sort_key ? 1 : a.index-b.index})
    var selected={};s.files.forEach(function(row){selected[row.index]=s.included[row.index]===undefined ? row.included!==false : s.included[row.index]})
    s.included=selected;s.phase="ready";publish()
  }
  function metadata(row) {
    if(!row || ["committing","keeping","discarding","cancel_wait"].indexOf(s.phase)>=0)return
    if(adopting){adopting=false;s.origin=row.origin || s.origin;settings({seed:row.seed,save_path:row.save_path})}
    s.name=row.name || s.name;s.size=row.size
    if(row.size!==undefined && !filesRequested){
      filesRequested=true
      if(service.client.caps.indexOf("subscribe")>=0){service.addTopics=["torrent:"+s.hash+".files"];s.phase="files_loading"}
      else {var generation=epoch;service.client.request("torrents.files",{hash:s.hash},{callback:function(result,failure){if(generation!==epoch || !s.visible)return;if(result)fileRows(result.files);else if(failure)error(failure.message)}})}
    }
    publish()
  }
  function inspect() {
    if(!s.hash || readPending || !service.online)return
    readPending=true;var generation=epoch
    service.client.request("torrents.get",{hash:s.hash},{callback:function(result,failure){
      if(generation!==epoch)return
      readPending=false
      if(result)metadata(result.torrent);else if(failure)error(failure.message)
    }})
  }
  function staged(result) {
    s.hash=result.hash || s.hash
    if(result.duplicate){s.duplicate=true;s.owned=false;s.phase="duplicate";publish();return}
    if(!s.hash){error("The daemon did not return a torrent hash");return}
    s.owned=true;s.phase="resolving"
    if(s.cancelRequested){f.cancel();return}
    if(s.commitRequested)f.commit();else inspect();publish()
  }
  function settings(options) {
    var generation=epoch
    function apply(value){
      if(generation!==epoch || !s.visible)return
      s.seed=false
      if(!pathDirty){var path=options.save_path || value.save_path || "";if(path!==s.savePath || !s.space.supported){s.savePath=path;scheduleSpace(true)}}
      publish()
    }
    apply(service.store.settings || {})
    if(service.client.caps.indexOf("settings")>=0)service.client.request("app.settings.get",{}, {callback:function(result){if(result)apply(result.settings || result)}})
  }
  function stageSource(){
    if(!service.online){s.phase="connecting";publish();return}
    if(s.phase==="connecting")settings({save_path:s.requestedPath})
    s.stageSupported=service.client.caps.indexOf("staging")>=0
    if(!s.stageSupported){s.phase="preview";publish();return}
    s.phase="staging";send("torrents.add",{source:s.source,stage:true,seed:false,origin:s.origin},"staging",staged)
  }
  f.open=function(source,options){
    options=options || {}
    if(s.visible && s.phase!=="input"){service.notify("Finish or discard the current add first");return false}
    epoch++;reset();s.visible=true;s.phase="input";s.origin=options.origin || "manual";s.category=options.category;s.stopped=options.stopped===true;s.requestedPath=options.save_path
    if(["manual","handler","agent"].indexOf(s.origin)<0){error("Unsupported add origin");return false}
    settings(options)
    if(!source){publish();return true}
    var parsed=Source.normalize(source)
    if(!parsed){error("Enter a magnet, HTTP URL or local .torrent path");return false}
    s.source=parsed.source;s.hash=parsed.hash;s.name=Model.sourceName(s.source)
    var existing=s.hash && service.store.torrents[s.hash]
    if(existing){s.name=existing.name;s.duplicate=true;s.phase="duplicate";publish();return true}
    s.stageSupported=service.client.caps.indexOf("staging")>=0
    stageSource()
    publish();return true
  }
  // Search owns provider resolution. The same review sheet delays a legacy
  // grab until Download, or receives the real staged hash when negotiated.
  f.search=function(row,session){
    if(s.visible){service.notify("Finish or discard the current add first");return false}
    epoch++;reset();s.visible=true;s.phase="preview";s.origin="search";s.name=row.name;s.size=row.size
    s.stageSupported=service.client.caps.indexOf("staging")>=0
    s.searchArgs=s.stageSupported && session ? {search_id:session,key:row.key} : {result:row}
    settings({})
    if(s.stageSupported){s.phase="staging";send("search.grab",Object.assign({},s.searchArgs,{stage:true,seed:false}),"search",staged)}
    publish();return true
  }
  // Shared Shows/RSS entry: stage first, mark the source only on commit.
  f.release=function(op,args,row){
    if(s.visible){service.notify("Finish or discard the current add first");return false}
    if(["shows.grab","rss.grab"].indexOf(op)<0 || service.client.caps.indexOf("staging")<0){service.notify("Staged review is unavailable");return false}
    epoch++;reset();s.visible=true;s.phase="staging";s.origin=op==="shows.grab" ? "shows" : "rss";s.name=row.title || "Release";s.size=row.size
    s.stageSupported=true;s.grabOp=op;s.grabArgs=Object.assign({},args);settings({})
    send(op,Object.assign({},args,{stage:true,seed:false}),s.origin,staged);publish();return true
  }
  f.review=function(text){var parsed=Source.parse(text);if(!parsed){error("Enter a magnet, HTTP URL or local .torrent path");return false}s.phase="input";return f.open(parsed.source,{origin:s.origin,seed:s.seed,save_path:s.savePath,category:s.category,stopped:s.stopped})}
  f.existing=function(hash){
    if(s.visible)return false
    epoch++;reset();s.visible=true;s.hash=hash;s.phase="resolving";s.stageSupported=true;s.owned=true
    adopting=true;inspect();publish();return true
  }
  f.toggle=function(index){if(busy())return;var next=Object.assign({},s.included);next[index]=!next[index];s.included=next;publish()}
  f.path=function(value){if(busy())return;s.commitRequested=false;pathDirty=true;s.savePath=value;scheduleSpace(false);publish()}
  f.checkPath=function(){checkSpace()}
  f.retry=function(){s.error="";if(s.grabArgs && !s.owned){s.phase="staging";send(s.grabOp,Object.assign({},s.grabArgs,{stage:true,seed:false}),s.origin,staged)}else if(s.searchArgs && s.stageSupported && !s.owned){s.phase="staging";send("search.grab",Object.assign({},s.searchArgs,{stage:true,seed:false}),"search",staged)}else{s.phase=s.owned ? "resolving" : s.searchArgs ? "preview" : "input";if(s.owned)inspect()}publish()}
  f.commit=function(){
    if(s.duplicate){service.finishAdd(s.hash);close();return}
    if(["preview","ready","connecting","staging","resolving","files_loading","metadata_timeout"].indexOf(s.phase)<0)return
    if(s.savePath && s.savePath[0]!=="/"){error("Enter an absolute save folder");return}
    if(!spaceReady())return
    s.commitRequested=true
    if(s.phase==="connecting" || s.phase==="staging"){publish();return}
    var args={seed:false},op="torrents.add"
    if(s.savePath)args.save_path=s.savePath
    if(s.owned){
      args.hash=s.hash;op="torrents.commit"
      if(s.files.length)args.files=s.files.filter(function(row){return s.included[row.index]}).map(function(row){return row.index})
      if(s.files.length && !args.files.length){error("Select at least one file");return}
    }else if(s.searchArgs){op="search.grab";args=Object.assign({},s.searchArgs,args)}
    else{args.source=s.source;args.origin=s.origin;if(s.stopped)args.stopped=true;if(s.category!==undefined)args.category=s.category}
    s.phase="committing"
    send(op,args,s.owned ? "staging" : s.searchArgs ? "search" : "",function(result){var hash=result.hash || s.hash;service.notify(result.duplicate ? "Already in your list" : "Download added");service.finishAdd(hash);close()})
    publish()
  }
  f.cancel=function(){
    if(s.phase==="committing" || s.phase==="discarding")return
    if(s.owned && s.hash){s.phase="discarding";send("torrents.discard",{hash:s.hash},"staging",function(){close()});publish()}
    else if(s.phase==="staging" || s.phase==="cancel_wait"){s.cancelRequested=true;s.phase="cancel_wait";publish()}
    else close()
  }
  f.keep=function(){if(!s.hash)return;s.phase="keeping";send("torrents.stop",{hashes:[s.hash]},"",function(){close()});publish()}
  f.tick=function(){
    if(!s.visible)return
    if(s.phase==="connecting" && service.online)stageSource()
    if(spaceDue && service.now>=spaceDue)checkSpace()
    s.elapsed_s=Math.max(0,Math.floor((service.now-s.created)/1000))
    if(s.phase==="resolving" && s.elapsed_s>=90)s.phase="metadata_timeout"
    if(s.owned && s.phase==="resolving")inspect()
    publish()
  }
  f.frame=function(frame){
    if(!s.visible)return
    if(s.phase==="connecting" && service.online)stageSource()
    if(frame.topic==="events" && (frame.ev==="snapshot.end" || frame.ev==="delta") && s.pendingRef){
      var outcome=service.store.mutations[s.pendingRef]
      if(outcome)f.frame({ev:"mutation.result",ref:s.pendingRef,outcome:outcome.status,result:outcome.result,error:outcome.error})
    }
    if(frame.ev==="mutation.result" && s.pendingRef && frame.ref===s.pendingRef){
      if(frame.outcome==="pending")return
      record("event",s.pendingOp,frame)
      if(frame.outcome==="done"){var done=s.pendingDone;s.pendingRef="";done(frame.result || {})}else if(frame.outcome!=="done")error(frame.error ? frame.error.message : "Result unknown; refresh before trying again")
    }
    if(s.owned && frame.topic==="torrent:"+s.hash+".files" && (frame.ev==="snapshot.end" || frame.ev==="delta")){
      var files=service.store.topics[frame.topic] || {};fileRows(Object.keys(files).map(function(key){return files[key]}))
    }
  }
  f.snapshot=function(){return {visible:s.visible,phase:s.phase,source:s.source,origin:s.origin,search:!!(s.searchArgs || s.grabArgs),hash:s.hash,name:s.name,size:s.size,files:s.files,included:s.included,seed:s.seed,savePath:s.savePath,space:s.space,destinationReady:spaceReady(),neededBytes:requiredBytes(),insufficientSpace:tooLarge(),error:s.error,duplicate:s.duplicate,stopped:s.stopped===true,owned:s.owned,stageSupported:s.stageSupported,elapsed_s:s.elapsed_s,canDownload:["preview","ready","connecting","staging","resolving","files_loading","metadata_timeout"].indexOf(s.phase)>=0 && !s.commitRequested && spaceReady(),commitRequested:s.commitRequested,busy:busy(),trace:trace}}
  return f
}
