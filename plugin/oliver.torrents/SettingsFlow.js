.pragma library
.import "SettingsSchema.js" as Schema
function create(service,changed,clock){
 var now=clock || Date.now
 var f={pending:{},errors:{},banner:"",interfaces:[],active:false,loaded:false},lastSeq=0,restartAt=0,restartDone=false
 function equal(a,b){return a===b || (a===undefined && b===null)}
 function reconcile(){
  Object.keys(f.pending).forEach(function(key){var pending=f.pending[key];if(pending.acked && equal(service.store.settings[key],pending.value))delete f.pending[key]})
 }
 function events(){service.store.events.forEach(function(event){if(event.seq<=lastSeq)return;lastSeq=event.seq;if(restartAt && event.ev==="engine.changed" && event.state==="running")restartDone=true})}
 f.open=function(){
  f.active=true;f.banner="";changed()
  service.client.request("app.settings.get",{},{callback:function(result,error){if(error)f.errors.load=error.message;else f.loaded=true;changed()}})
  service.client.request("app.network",{},{callback:function(result,error){if(result)f.interfaces=result.interfaces;else if(error)f.errors.network=error.message;changed()}})
 }
 f.close=function(){f.active=false}
 f.set=function(key,input){
  if(f.pending[key])return false
  var value
  try{value=Schema.parse(Schema.field(key),input)}catch(error){f.errors[key]=String(error.message || error);changed();return false}
  delete f.errors[key];f.pending[key]={value:value,acked:false};changed()
  var args={};args[key]=value
  var ref=service.mutate("app.settings.set",args,"settings",function(result,error){
   if(error){delete f.pending[key];f.errors[key]=error.message}
   else if(f.pending[key]){
    f.pending[key].acked=true
    if(result.restart){restartAt=now();restartDone=false;f.banner="applies after restart; restarting now"}
    reconcile()
   }
   changed()
  })
  if(!ref){delete f.pending[key];f.errors[key]=service.lastError || "Could not send setting";changed();return false}
  return ref
 }
 f.frame=function(frame){
  if(frame.topic==="events" && ["delta","snapshot.end"].indexOf(frame.ev)>=0)events()
  else if(frame.ev==="engine.changed"){if(restartAt && frame.state==="running")restartDone=true}
  if(frame.topic==="settings" || (frame.id!==undefined && Object.keys(f.pending).length)){reconcile();changed()}
 }
 f.nextWakeup=function(){return restartAt && restartDone ? restartAt+1500 : Infinity}
 f.tick=function(){if(now()>=f.nextWakeup()){restartAt=0;f.banner="Settings applied after restart";changed()}}
 f.snapshot=function(){return {values:service.store.settings,pending:f.pending,errors:f.errors,banner:f.banner,interfaces:f.interfaces,loaded:f.loaded}}
 return f
}
