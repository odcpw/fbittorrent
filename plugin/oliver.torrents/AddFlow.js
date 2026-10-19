.pragma library
.import "AddSource.js" as Source
.import "Model.js" as Model
function create(service,changed,debug) {
  var f={},epoch=0,trace=[],s={},readPending=false,filesRequested=false,seedDirty=false,pathDirty=false,spaceSerial=0,spaceDue=0
  function publish(){changed()}
  function record(kind,op,data){if(debug){trace.push({kind:kind,op:op,data:data});if(trace.length>32)trace.shift()}}
  function reset(){s={visible:false,phase:"closed",source:null,origin:"manual",hash:"",name:"",files:[],included:{},seed:false,savePath:"",error:"",duplicate:false,owned:false,stageSupported:false,created:service.now,elapsed_s:0,pendingRef:"",cancelRequested:false,space:{supported:false,pending:false}};readPending=false;filesRequested=false;seedDirty=false;pathDirty=false;spaceDue=0}
  reset()
  function spaceReady(){return !s.space.supported || (!s.space.pending && s.space.writable===true && s.space.path===s.savePath)}
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
      publish()
    }})
    publish()
  }
  function scheduleSpace(immediate){
    if(service.client.caps.indexOf("free_space")<0)return
    spaceSerial++;s.space={supported:true,path:s.savePath,pending:true};spaceDue=service.now+250
    if(immediate)checkSpace()
  }
  function busy(){return ["staging","committing","discarding","cancel_wait","keeping"].indexOf(s.phase)>=0}
  function error(message){s.error=message || "No answer; refresh before trying again";s.phase="error";publish()}
  function clearTopics(){service.addTopics=[]}
  function close(){epoch++;s.visible=false;s.phase="closed";s.pendingRef="";clearTopics();publish()}
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
    inspect();publish()
  }
  function settings(options) {
    var generation=epoch
    function apply(value){
      if(generation!==epoch || !s.visible)return
      if(!seedDirty)s.seed=options.seed===undefined ? value.seed_by_default===true : options.seed===true
      if(!pathDirty){var path=options.save_path || value.save_path || "";if(path!==s.savePath || !s.space.supported){s.savePath=path;scheduleSpace(true)}}
      publish()
    }
    apply(service.store.settings || {})
    if(service.client.caps.indexOf("settings")>=0)service.client.request("app.settings.get",{}, {callback:function(result){if(result)apply(result.settings || result)}})
  }
  f.open=function(source,options){
    options=options || {}
    if(s.visible && s.phase!=="input"){service.notify("Finish or discard the current add first");return false}
    epoch++;reset();s.visible=true;s.phase="input";s.origin=options.origin || "manual";s.category=options.category;s.stopped=options.stopped===true
    if(["manual","handler","agent"].indexOf(s.origin)<0){error("Unsupported add origin");return false}
    settings(options)
    if(!source){publish();return true}
    var parsed=Source.normalize(source)
    if(!parsed){error("Enter a magnet, HTTP URL or local .torrent path");return false}
    s.source=parsed.source;s.hash=parsed.hash;s.name=Model.sourceName(s.source)
    var existing=s.hash && service.store.torrents[s.hash]
    if(existing){s.name=existing.name;s.duplicate=true;s.phase="duplicate";publish();return true}
    s.stageSupported=service.client.caps.indexOf("staging")>=0
    if(s.stageSupported){s.phase="staging";send("torrents.add",{source:s.source,stage:true,origin:s.origin},"staging",staged)}
    else s.phase="preview"
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
    if(s.stageSupported){s.phase="staging";send("search.grab",Object.assign({},s.searchArgs,{stage:true}),"search",staged)}
    publish();return true
  }
  f.review=function(text){var parsed=Source.parse(text);if(!parsed){error("Enter a magnet, HTTP URL or local .torrent path");return false}s.phase="input";return f.open(parsed.source,{origin:s.origin,seed:s.seed,save_path:s.savePath,category:s.category,stopped:s.stopped})}
  f.existing=function(hash){
    if(s.visible)return false
    epoch++;reset();s.visible=true;s.hash=hash;s.phase="resolving";s.stageSupported=true;s.owned=true
    inspect();settings({});publish();return true
  }
  f.toggle=function(index){if(busy())return;var next=Object.assign({},s.included);next[index]=!next[index];s.included=next;publish()}
  f.seed=function(value){if(busy())return;seedDirty=true;s.seed=!!value;publish()}
  f.path=function(value){if(busy())return;pathDirty=true;s.savePath=value;scheduleSpace(false);publish()}
  f.checkPath=function(){checkSpace()}
  f.retry=function(){s.error="";if(s.searchArgs && s.stageSupported && !s.owned){s.phase="staging";send("search.grab",Object.assign({},s.searchArgs,{stage:true}),"search",staged)}else{s.phase=s.owned ? "resolving" : s.searchArgs ? "preview" : "input";if(s.owned)inspect()}publish()}
  f.commit=function(){
    if(s.duplicate){service.finishAdd(s.hash);close();return}
    if(s.phase!=="preview" && s.phase!=="ready")return
    if(s.savePath && s.savePath[0]!=="/"){error("Enter an absolute save folder");return}
    if(!spaceReady())return
    var args={seed:s.seed},op="torrents.add"
    if(s.savePath)args.save_path=s.savePath
    if(s.owned){
      args.hash=s.hash;args.files=s.files.filter(function(row){return s.included[row.index]}).map(function(row){return row.index});op="torrents.commit"
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
    if(spaceDue && service.now>=spaceDue)checkSpace()
    s.elapsed_s=Math.max(0,Math.floor((service.now-s.created)/1000))
    if(s.phase==="resolving" && s.elapsed_s>=90)s.phase="metadata_timeout"
    if(s.owned && s.phase==="resolving")inspect()
    publish()
  }
  f.frame=function(frame){
    if(!s.visible)return
    if(frame.topic==="events" && (frame.ev==="snapshot.end" || frame.ev==="delta") && s.pendingRef){
      var outcome=service.store.mutations[s.pendingRef]
      if(outcome)f.frame({ev:"mutation.result",ref:s.pendingRef,outcome:outcome.status,result:outcome.result,error:outcome.error})
    }
    if(frame.ev==="mutation.result" && s.pendingRef && frame.ref===s.pendingRef){
      record("event",s.pendingOp,frame)
      if(frame.outcome==="done"){var done=s.pendingDone;s.pendingRef="";done(frame.result || {})}else if(frame.outcome!=="done")error(frame.error ? frame.error.message : "Result unknown; refresh before trying again")
    }
    if(s.owned && frame.topic==="torrent:"+s.hash+".files" && (frame.ev==="snapshot.end" || frame.ev==="delta")){
      var files=service.store.topics[frame.topic] || {};fileRows(Object.keys(files).map(function(key){return files[key]}))
    }
  }
  f.snapshot=function(){return {visible:s.visible,phase:s.phase,source:s.source,origin:s.origin,search:!!s.searchArgs,hash:s.hash,name:s.name,size:s.size,files:s.files,included:s.included,seed:s.seed,savePath:s.savePath,space:s.space,destinationReady:spaceReady(),error:s.error,duplicate:s.duplicate,stopped:s.stopped===true,owned:s.owned,stageSupported:s.stageSupported,elapsed_s:s.elapsed_s,busy:busy(),trace:trace}}
  return f
}
