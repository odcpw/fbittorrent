.pragma library
.import "Model.js" as Model
function describe(s) {
  if(!s)return {message:"fbittorrent is not running",banner:"",urgent:false,dim:false}
  var client=s.client || {},hello=client.hello || {},summary=s.store.summary || {},daemon=summary.daemon || {},info=s.store.info || {},network=summary.network || s.store.network || {}
  var r={message:"",banner:"",urgent:false,dim:!s.online,idle:""}
  if(client.protocolMismatch){r.banner="This plugin speaks API 1; the running fbittorrent speaks API "+(hello.proto || "another version")+". Update the plugin or run `fbittorrent restart`.";r.urgent=true}
  else if(s.startupRequested)r.message="Starting…"+(s.torrents.length ? " checking "+s.torrents.length+" torrents" : "")
  else if(s.reconnecting){r.message="Reconnecting… retry in "+Math.ceil((client.retryDelay || 500)/1000)+" s";r.dim=true}
  else if(!s.online)r.message=s.stoppedRecently ? "fbittorrent stopped. Enter Start · w Open window" : client.mode==="connecting" && s.now-client.started<2000 ? "Connecting…" : "fbittorrent is not running. Enter Start · w Open window"
  if(s.online && (s.backendState==="parked" || network.ready===false || network.state==="waiting")){
    r.banner="Waiting for ProtonVPN ("+(network.interface || network.selection || "proton0")+"). Downloads resume when it connects.";r.urgent=true;r.dim=true
  } else if(s.online && s.backendState==="error"){r.banner=daemon.message || info.message || s.lastError || "Engine failed";r.urgent=true}
  else if(info.upgrade_pending || daemon.upgrade_pending)r.banner="Update installed; restarts when idle"
  else if(info.installed_version && (hello.server || {}).version && info.installed_version!==(hello.server || {}).version)r.banner="fbittorrent "+(hello.server || {}).version+" runs; "+info.installed_version+" is installed. Restart to update."
  if(s.online && s.torrents.length===0 && !r.message)r.message="Nothing here yet. Copy a magnet link and press a, or press / to search."
  if(summary.disk && summary.disk.low){r.banner="Low disk space. Downloads may pause until space is available.";r.urgent=true}
  if(daemon.idle_exit_in_s!==undefined && daemon.idle_exit_in_s!==null)r.idle="Idle · stops in "+Math.ceil(daemon.idle_exit_in_s/60)+" min"
  return r
}
function meta(t,parked) {
  if(t.error)return t.error
  var parts=[Model.stateLabel(t)]
  if(!parked && t.progress!==undefined && t.progress!==null)parts.push(Model.percent(t.progress,t.state==="done" || t.state==="seeding"))
  if(t.dl_bps!==undefined)parts.push("↓ "+Model.rate(t.dl_bps))
  if(t.eta_s!==undefined)parts.push(Model.eta(t.eta_s,false))
  if(t.peers!==undefined)parts.push(t.peers+" peers")
  if(t.seeds!==undefined)parts.push(t.seeds+" seeds")
  if(t.playable && t.playable.ready){if(t.playable.buffered_s!==undefined)parts.push(Model.eta(t.playable.buffered_s,false)+" buffered");if(t.playable.stall_free===false){parts.push("may stall");if(t.playable.finishes_in_s!==undefined)parts.push("finishes in "+Model.eta(t.playable.finishes_in_s,false))}}
  return parts.join(" · ")
}
function verb(action) {
  return {"torrents.stop":"pausing…","torrents.start":"starting…","torrents.remove":"removing…","torrents.set_seed":"setting seed policy…"}[action.op] || "working…"
}
