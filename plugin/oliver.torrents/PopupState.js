.pragma library
.import "Model.js" as Model
function diskBanner(disk) {
  if(!disk || !disk.low)return ""
  return "Not enough space for the running downloads: "+Model.size(disk.needed_bytes,false)+" needed, "+(disk.free_bytes===undefined ? "free space unknown" : Model.size(disk.free_bytes,false)+" free")
}
function idle(daemon,online){return online && daemon && daemon.idle_exit_in_s!==undefined && daemon.idle_exit_in_s!==null ? "Idle · stops in "+Math.ceil(daemon.idle_exit_in_s/60)+" min" : ""}
function describe(s) {
  if(!s)return {message:"fbittorrent is not running",banner:"",urgent:false,dim:false}
  var client=s.client || {},summary=s.store.summary || {},daemon=summary.daemon || {},info=s.store.info || {},network=summary.network || s.store.network || {}
  var r={message:"",banner:"",urgent:false,dim:!s.online,idle:""}
  if(s.startupRequested)r.message="Starting…"+(s.torrents.length ? " checking "+s.torrents.length+" torrents" : "")
  else if(s.reconnecting){r.message="Reconnecting…";r.dim=true}
  else if(!s.online)r.message=s.stoppedRecently ? "fbittorrent stopped. Enter Start · w Open window" : client.mode==="connecting" && s.now-client.started<2000 ? "Connecting…" : "fbittorrent is not running. Enter Start · w Open window"
  if(s.online && (s.backendState==="parked" || network.ready===false || network.state==="waiting")){
    r.banner="Waiting for ProtonVPN ("+(network.interface || network.selection || "proton0")+"). Downloads resume when it connects.";r.urgent=true;r.dim=true
  } else if(s.online && s.backendState==="error"){r.banner=daemon.message || info.message || s.lastError || "Engine failed";r.urgent=true}
  else if(info.upgrade_pending || daemon.upgrade_pending)r.banner="Update installed; restarts when idle"
  if(s.online && s.torrents.length===0 && !r.message)r.message="Nothing here yet. Copy a magnet link and press a, or press / to search."
  if(summary.disk && summary.disk.low){r.banner=diskBanner(summary.disk);r.urgent=true}
  r.idle=idle(daemon,s.online)
  return r
}
function meta(t,parked) {
  if(t.error)return t.error
  var parts=[Model.stateLabel(t)]
  if(!parked && t.progress!==undefined && t.progress!==null)parts.push(Model.percent(t.progress,t.state==="done" || t.state==="seeding"))
  if(t.dl_bps!==undefined)parts.push("↓ "+Model.rate(t.dl_bps))
  if(t.eta_s!==undefined)parts.push(Model.eta(t.eta_s,false))
  if(t.peers!==undefined || t.seeds!==undefined)parts.push(Model.peerCount(t,false))
  if(t.playable && t.playable.ready){if(t.playable.buffered_s!==undefined)parts.push(Model.eta(t.playable.buffered_s,false)+" buffered");if(t.playable.stall_free===false){parts.push("may stall");if(t.playable.finishes_in_s!==undefined)parts.push("finishes in "+Model.eta(t.playable.finishes_in_s,false))}}
  return parts.join(" · ")
}
function verb(action) {
  return {"torrents.stop":"pausing…","torrents.start":"starting…","torrents.remove":"removing…","torrents.recheck":"checking files…","torrents.set_seed":"setting upload permission…"}[action.op] || "working…"
}

function group(t){return t.staged ? "reviewing" : Model.isActive(t) ? "live" : (t.state==="seeding" || t.state==="queued" || t.state==="paused" || t.state==="done") ? t.state : "other"}
function dashboardFilter(group) {return group==="live" ? "downloading" : group==="other" ? "all" : group}
