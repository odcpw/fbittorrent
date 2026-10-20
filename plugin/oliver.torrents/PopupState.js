.pragma library
.import "Model.js" as Model
function diskBanner(disk) {
  if(!disk || !disk.low)return ""
  return "Not enough space for the running downloads: "+Model.size(disk.needed_bytes,false)+" needed, "+(disk.free_bytes===undefined ? "free space unknown" : Model.size(disk.free_bytes,false)+" free")
}
function idle(daemon,online){return online && daemon && daemon.idle_exit_in_s!==undefined && daemon.idle_exit_in_s!==null ? "Idle · stops in "+Math.ceil(daemon.idle_exit_in_s/60)+" min" : ""}
// The network the owner waits for, by the name the user configured.
function networkName(network) {
  var n=network || {},iface=n.interface || n.selection
  if(n.selection==="proton" || (!n.selection && (!iface || iface==="proton0")))return "ProtonVPN"
  return iface
}
function warningBanner(warnings) {
  if(!warnings || !warnings.length)return ""
  var first=warnings[0],file={config:"config.toml",shows:"shows.json",rss:"rss.json"}[first.source] || first.source
  if(first.source==="config" && /^(config\.toml|Unknown setting|Invalid config\.toml)/.test(first.message || ""))return first.message+" Keeping the last valid settings."
  return file+" is unreadable; "+(first.source==="config" ? "using the last valid settings" : "showing the last loaded list")+
    (warnings.length>1 ? " (+"+(warnings.length-1)+" more)" : "")+". Fix the file to clear this."
}
function describe(s) {
  if(!s)return {message:"fbittorrent stopped",banner:"",urgent:false,dim:false}
  var client=s.client || {},summary=s.store.summary || {},daemon=summary.daemon || {},info=s.store.info || {},network=summary.network || s.store.network || {}
  var r={message:"",banner:"",urgent:false,dim:!s.online,idle:""}
  if(client.protocolMismatch)r.message=client.lastError ? client.lastError.message : "Update the plugin with fbittorrent setup --plugin=copy, then restart the shell."
  else if(s.startupRequested)r.message="Starting…"+(s.torrents.length ? " checking "+s.torrents.length+" torrents" : "")
  else if(s.reconnecting){r.message="Reconnecting…";r.dim=true}
  else if(!s.online)r.message=!s.stoppedRecently && client.mode==="connecting" && s.now-client.started<2000 ? "Connecting…" : "fbittorrent stopped. Enter Start · w Open window"
  if(s.online && (s.backendState==="parked" || network.ready===false || network.state==="waiting")){
    var name=networkName(network)
    r.banner="Waiting for "+(name==="ProtonVPN" ? "ProtonVPN ("+(network.interface || "proton0")+")" : name)+". Connect it or change the VPN interface in Settings. Downloads resume when it connects.";r.urgent=true;r.dim=true
  } else if(s.online && s.backendState==="error"){r.banner=daemon.message || info.message || s.lastError || "Engine failed";r.urgent=true}
  else if(s.online && (info.upgrade_pending || daemon.upgrade_pending))r.banner="Update installed; restarts when idle"
  else if(s.online && daemon.warnings && daemon.warnings.length)r.banner=warningBanner(daemon.warnings)
  if(s.online && s.torrents.length===0 && !r.message)r.message="Nothing here yet. Copy a magnet link and press a, or press / to search."
  if(s.online && summary.disk && summary.disk.low){r.banner=diskBanner(summary.disk);r.urgent=true}
  r.idle=idle(daemon,s.online)
  return r
}
function meta(t,parked) {
  if(Model.problem(t))return Model.problem(t)
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
