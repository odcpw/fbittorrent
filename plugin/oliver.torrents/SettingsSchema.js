.pragma library
var fields=[
 {key:"max_active_downloads",label:"Active downloads",kind:"number"},
 {key:"save_path",label:"Download folder",kind:"text"},
 {key:"upload",label:"Upload",kind:"choice",options:["off","on"]},
 {key:"idle_exit_secs",label:"Idle exit (seconds;0 disables)",kind:"number"},
 {key:"peer_encryption",label:"Peer encryption",kind:"choice",options:["off","prefer","require"]},
 {key:"network_interface",label:"Network binding",kind:"interface"},
 {key:"peer_port",label:"Peer port (0 assigns a port)",kind:"number"},
 {key:"download_limit_bps",label:"Download limit (bytes/s)",kind:"limit"},
 {key:"upload_limit_bps",label:"Upload limit (bytes/s)",kind:"limit"},
 {key:"open_command",label:"Open command",kind:"text"},
 {key:"handler",label:"Magnet handler",kind:"choice",options:["add","confirm"]},
 {key:"show_quality",label:"Default show quality",kind:"text"},
 {key:"notify",label:"Notifications",kind:"bool"},
 {key:"autostart",label:"Start daemon on a write",kind:"bool"},
 {key:"clipboard_watch",label:"Watch clipboard (opt-in)",kind:"bool"},
 {key:"reduce_motion",label:"Reduce motion",kind:"bool"},
 {key:"timeout_secs",label:"Request timeout (seconds)",kind:"number"}
]
function field(key){return fields.find(function(row){return row.key===key})}
function parse(spec,value){
 if(!spec)throw Error("Unknown setting")
 if(spec.kind==="bool"){if(typeof value!=="boolean")throw Error("Choose on or off");return value}
 var text=String(value).trim()
 if(spec.kind==="interface")return text || null
 if(spec.kind==="choice"){if(spec.options.indexOf(text)<0)throw Error("Choose "+spec.options.join(" / "));return text}
 if(spec.kind==="limit"){
  if(!text)return null
  var match=/^(\d+)([kKmM]?)$/.exec(text)
  if(!match)throw Error("Use bytes/s, K or M; leave empty for unlimited")
  var n=Number(match[1])*Math.pow(1024,match[2].toLowerCase()==="k" ? 1 : match[2].toLowerCase()==="m" ? 2 : 0)
  if(!Number.isSafeInteger(n) || n>4294967295 || (n!==0 && n<16384))throw Error("Use 16384..4294967295 bytes/s, or empty for unlimited")
  return n===0 ? null : n
 }
 if(spec.kind==="number"){
  if(!/^\d+$/.test(text) || !Number.isSafeInteger(Number(text)))throw Error("Use a whole nonnegative number")
  var number=Number(text)
  if((spec.key==="max_active_downloads" || spec.key==="timeout_secs") && number<1)throw Error("Must be at least 1")
  if(spec.key==="idle_exit_secs" && number!==0 && number<30)throw Error("Use 0 or at least 30 seconds")
  if(spec.key==="peer_port" && number>65535)throw Error("Use 0..65535")
  return number
 }
 if(!text)throw Error("This setting must not be empty")
 return text
}
function text(value){return value===undefined || value===null ? "" : String(value)}
