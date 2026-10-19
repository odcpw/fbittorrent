.pragma library

// Retain only a confirmed keep-files removal, bounded by the daemon's deadline.
// Failed/uncertain requests are never presented as undoable or resent.
function create(service,changed){
  var f={hashes:[],until:0,busy:false},pending={}
  function publish(){changed()}
  function finish(ref,result,error){
    var entry=pending[ref];if(!entry)return
    if(error){delete pending[ref];f.busy=false;service.notify(error.message);publish();return}
    if(result && result.state==="pending")return
    delete pending[ref];f.busy=false
    if(entry.restore){f.hashes=f.hashes.filter(function(h){return h!==entry.hash});service.notify("Torrent restored");restoreNext()}
    else if(!entry.files && result && result.undo_until){f.hashes=entry.hashes;f.until=result.undo_until*1000;service.notify("Removed, files kept · Undo available for 10 seconds")}
    else service.notify("Torrent records and files removed")
    publish()
  }
  f.remove=function(hashes,files){
    if(f.busy || !hashes.length)return false
    f.busy=true
    var captured=hashes.slice(),ref=service.mutate("torrents.remove",{hashes:captured,delete_files:!!files},null,function(result,error){finish(ref,result,error)})
    if(ref)pending[ref]={hashes:captured,files:!!files}
    else f.busy=false
    publish();return ref
  }
  f.available=function(now){return !f.busy && f.hashes.length>0 && now<f.until}
  // Continue from either an immediate reply or its later mutation result.
  function restoreNext(){
    if(!f.hashes.length || Date.now()>=f.until){f.busy=false;publish();return}
    f.busy=true
    var hash=f.hashes[0],ref=service.mutate("torrents.add",{source:{restore:hash}},null,function(result,error){finish(ref,result,error)})
    if(ref)pending[ref]={restore:true,hash:hash};else f.busy=false
    publish()
  }
  f.undo=function(){if(!f.available(Date.now()))return false;restoreNext();return true}
  f.reset=function(){pending={};f.hashes=[];f.until=0;f.busy=false;publish()}
  function event(e){if(e.ev==="mutation.result")finish(e.ref,e.result,e.outcome==="done" ? null : e.error || {message:"Removal was not confirmed"})}
  f.frame=function(frame){if(frame.ev==="mutation.result")event(frame);if(frame.topic==="events"){if(frame.ev==="snapshot.chunk")(frame.items || []).forEach(event);if(frame.ev==="delta")(frame.added || []).forEach(event)}}
  return f
}
