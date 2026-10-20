.pragma library
.import "Model.js" as Model
function reason(row,network){
  if(!row)return ""
  if(Model.problem(row))return Model.problem(row)
  return {user:"paused by you",queue_full:"waiting for a download slot",complete_no_seed:"complete; upload is off",vpn_down:"waiting for "+(network.interface || network.selection || "VPN"),restore_failed:"could not restore: "+(row.error || "check Log and retry"),metadata_timeout:"no peers answered",missing_files:"files are missing",disk_full:"disk full"}[row.reason] || row.error || row.reason || ""
}
function policy(row){return "Upload: "+(row.upload_enabled===true ? "on" : "off")}
function playable(p){if(!p)return "";return (p.ready ? "Ready to watch" : "Not ready to watch")+(p.buffered_s!==undefined ? " · "+Model.eta(p.buffered_s,false)+" buffered" : "")+(p.stall_free===false ? " · may stall" : "")}
function localUrl(path){return path ? "file://"+String(path).split("/").map(encodeURIComponent).join("/") : ""}

function networking(row){return !row || row.private===undefined || row.size===undefined ? "" : row.private ? "Private torrent (no DHT, no PeX)" : "Public torrent"}
