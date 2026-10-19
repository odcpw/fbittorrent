.pragma library
.import "SettingsSchema.js" as Schema

function effectiveCap(row,settings,alt){
  if(settings.max_active_downloads===undefined && settings.download_limit_bps===undefined && settings.alt_download_limit_bps===undefined)return row.effective_download_limit_bps
  var rates=[row.download_limit_bps,alt ? settings.alt_download_limit_bps : settings.download_limit_bps].filter(function(n){return typeof n==="number" && n>0})
  return rates.length ? Math.min.apply(Math,rates) : null
}
function parseCap(text){return Schema.parse({kind:"limit"},/^(none|unlimited)$/i.test(String(text).trim()) ? "" : text)}
// The shared service owns requests across details-panel reloads. A timed-out
// move stays unconfirmed; only reads or its original result can resolve it.
function create(service,changed){
  var tasks={},notices={},f={hash:"",modal:"",message:""}
  function publish(){changed()}
  function keep(hash,text){if(!notices[hash] && Object.keys(notices).length>=32)delete notices[Object.keys(notices)[0]];notices[hash]=text;publish()}
  function task(hash){for(var ref in tasks)if(tasks[ref].hash===hash)return tasks[ref];return null}
  function read(hash,done){service.client.request("torrents.get",{hash:hash},{callback:function(result,error){if(error){keep(hash,error.message);return}if(done)done(result.torrent);service.refresh()}})}
  function finished(ref,result,error){
    var row=tasks[ref];if(!row)return
    if(error){
      if(error.code==="timeout"){row.unknown=true;keep(row.hash,"Outcome unconfirmed. Refresh the location; the request will not be sent again.");return}
      delete tasks[ref];keep(row.hash,error.message);return
    }
    if(result && result.state==="pending")return
    delete tasks[ref];keep(row.hash,row.op==="torrents.move" ? "Move complete" : "Download cap applied");read(row.hash)
  }
  function send(op,args,cap){
    var hash=f.hash
    if(!hash || task(hash) || Object.keys(tasks).length>=8)return false
    f.modal="";f.message="";keep(hash,op==="torrents.move" ? "Moving…" : "Applying download cap…")
    var ref=service.mutate(op,args,cap,function(result,error){finished(ref,result,error)})
    if(ref)tasks[ref]={hash:hash,op:op,path:args.save_path,bps:args.bps,unknown:false}
    else keep(hash,service.lastError || "Request could not be sent")
    publish();return ref
  }
  f.open=function(hash,modal){if(task(hash))return false;f.hash=hash;f.modal=modal;f.message="";publish();return true}
  f.cancel=function(){f.modal="";f.message="";publish()}
  f.move=function(path){if(typeof path!=="string" || path[0]!=="/" || /[\x00-\x1f\x7f]/.test(path)){f.message="Choose an absolute folder path";publish();return false}return send("torrents.move",{hashes:[f.hash],save_path:path},"move")}
  f.limit=function(text){var bps;try{bps=parseCap(text)}catch(error){f.message=error.message;publish();return false}return send("torrents.set_download_limit",{hashes:[f.hash],bps:bps},"download_limit")}
  f.refresh=function(hash){read(hash,function(row){Object.keys(tasks).forEach(function(ref){
    var t=tasks[ref];if(t.hash!==hash || !t.unknown)return
    var confirmed=t.op==="torrents.move" ? t.path===row.save_path : t.bps===(row.download_limit_bps===undefined ? null : row.download_limit_bps)
    if(confirmed || t.oldInstance){delete tasks[ref];keep(hash,confirmed ? t.op==="torrents.move" ? "Move complete" : "Download cap applied" : "Daemon restarted. Confirmed location: "+row.save_path+". The old request was not sent again.")}
  })})}
  f.instanceChanged=function(){f.modal="";Object.keys(tasks).forEach(function(ref){var t=tasks[ref];t.unknown=true;t.oldInstance=true;keep(t.hash,"Daemon restarted. Refresh the location before another change.")});publish()}
  f.status=function(hash){return notices[hash] || ""}
  f.busy=function(hash){return !!task(hash)}
  function event(e){if(e.ev==="mutation.result")finished(e.ref,e.result,e.outcome==="done" ? null : e.error || {code:e.outcome,message:"Request failed"})}
  f.frame=function(frame){
    if(frame.ev==="mutation.result")event(frame)
    if(frame.topic==="events"){if(frame.ev==="snapshot.chunk")(frame.items || []).forEach(event);if(frame.ev==="delta")(frame.added || []).forEach(event)}
  }
  f.snapshot=function(){return {hash:f.hash,modal:f.modal,message:f.message}}
  return f
}
