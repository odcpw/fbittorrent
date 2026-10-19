.pragma library
.import "Glyphs.js" as Glyphs
.import "Model.js" as Model
.import "PopupState.js" as PopupState

function describe(svc) {
  svc=svc || {}
  var status=svc.status || {}, summary=svc.store ? svc.store.summary : {}, counts=summary.counts || (svc.store ? svc.store.counts : {})
  var result={state:"stopped", glyph:Glyphs.download, tone:"dim", rate:"", turtle:summary.alt===true,
    tooltip:"fbittorrent is not running · click to start"}
  if (!svc.online) {
    result.turtle=false
    if (svc.startupRequested) { result.state="starting"; result.tooltip="Starting fbittorrent…" }
    else if (svc.reconnecting) { result.state="reconnecting"; result.glyph=Glyphs.checking; result.tooltip="Reconnecting to fbittorrent…" }
    else if (svc.stoppedRecently) { result.state="stopped_recently"; result.tooltip=svc.byeReason==="idle" ? "fbittorrent stopped (idle)" : "fbittorrent stopped" }
    return result
  }
  var active=(counts.downloading || 0)+(counts.metadata || 0)+(counts.checking || 0)+(counts.stalled || 0)
  var error=svc.store && svc.store.firstError ? svc.store.firstError() : null
  var ready=svc.client && svc.client.caps.indexOf("watch")>=0 ? (summary.top || []).find(function(t) {
    return t.playable && t.playable.ready && t.playable.acked===false
  }) : null
  if (!ready && svc.client && svc.client.caps.indexOf("watch")>=0 && svc.store && svc.store.firstReady) ready=svc.store.firstReady()
  var dl=summary.dl_bps===undefined ? status.dl : summary.dl_bps
  var ul=summary.ul_bps===undefined ? status.up : summary.ul_bps
  if (summary.disk && summary.disk.low) {
    result.state="disk_low"; result.glyph=Glyphs.error; result.tone="urgent"; result.tooltip=PopupState.diskBanner(summary.disk)
  } else if (counts.error>0 || svc.backendState==="error") {
    result.state="error"; result.glyph=Glyphs.error; result.tone="urgent"
    result.tooltip=error && error.error ? error.error : counts.error>0 ? "Torrent error · " + counts.error + " needs attention" : svc.lastError || "Engine error"
  } else if (svc.backendState==="parked" || (summary.network && summary.network.state==="waiting")) {
    result.state="waiting_vpn"; result.glyph=Glyphs.vpn; result.tone="urgent"
    result.tooltip="Waiting for ProtonVPN" + (counts.waiting_vpn>0 ? " · " + counts.waiting_vpn + " downloads waiting" : "")
  } else if (ready) {
    result.state="ready"; result.glyph=Glyphs.ready; result.tone="normal"
    result.tooltip="Ready to watch: " + ready.name; result.rate=Model.barRate(dl,true)
  } else if (active>0) {
    result.state="downloading"; result.tone="normal"
    var words=[]
    ;["downloading","metadata","checking","stalled","queued"].forEach(function(kind) {if (counts[kind]>0) words.push(counts[kind]+" "+kind)})
    result.tooltip="↓ " + Model.barRate(dl,false) + " ↑ " + Model.barRate(ul,false) + " · " + words.join(" · ")
    result.rate=Model.barRate(dl,true)
  } else if (counts.seeding>0) {
    result.state="seeding"; result.glyph=Glyphs.upload; result.tone="normal"
    result.tooltip="↑ " + Model.barRate(ul,false) + " · " + counts.seeding + " seeding"; result.rate=Model.barRate(ul,true)
  } else {
    result.state="idle"
    var remaining=summary.daemon && summary.daemon.idle_exit_in_s
    result.tooltip=remaining!==undefined ? "Idle · stops in " + Math.ceil(remaining/60) + " min" : "Idle" + (counts.done>0 ? " · " + counts.done + " done" : "")
  }
  if (result.turtle) result.tooltip+=" · turtle limits on"
  result.tooltip=Model.plainTip(result.tooltip)
  return result
}
