.pragma library
// Offers are ephemeral. Initial event history seeds the cursor; only live
// events open a new offer. Re-subscribing never reopens a dismissed offer.
function create(service) {
  var lastSeq=0,instance="",lastHash="",f={}
  function reset(){lastSeq=0;lastHash="";if(service.clipboardOffer && service.clipboardOffer.watched)service.clipboardOffer=null}
  function identity(){var current=service.client ? service.client.instance : "";if(current!==instance){instance=current;reset()}}
  function consume(event,history){
    if(event.seq===undefined || event.seq<=lastSeq)return
    lastSeq=event.seq
    if(history || event.ev!=="clipboard.magnet" || event.already_added || !/^[0-9a-f]{40}$/.test(event.hash) || lastHash===event.hash)return
    lastHash=event.hash
    if(service.store.torrents[event.hash])return
    service.clipboardOffer={watched:true,hash:event.hash,name:event.name || "magnet link",source:{magnet:"magnet:?xt=urn:btih:"+event.hash}}
  }
  function sync(history){var events=service.store.events;for(var i=0;i<events.length;i++)consume(events[i],history)}
  f.frame=function(frame){
    if(!service.client || service.client.caps.indexOf("clipboard")<0)return
    identity()
    if(frame.ev==="clipboard.magnet")consume(frame,false)
    else if(frame.topic==="events" && frame.ev==="snapshot.end")sync(true)
    else if(frame.topic==="events" && frame.ev==="delta")sync(false)
  }
  f.reset=reset
  return f
}
