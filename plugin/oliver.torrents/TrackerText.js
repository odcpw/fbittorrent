.pragma library
function publicUrl(value) {
  if(!/^(https?|udp):\/\//i.test(String(value || "")))return "Unsupported tracker URL"
  return String(value || "").replace(/^([a-z][a-z0-9+.-]*:\/\/)[^/?#]*@/i,"$1").split(/[?#]/)[0]
}
function message(value,url) {
  var text=String(value || "").replace(/[\x00-\x1f\x7f]/g," "),raw=String(url || ""),secrets=[]
  var authority=(raw.match(/^[a-z][a-z0-9+.-]*:\/\/([^/?#]*)/i) || [])[1] || ""
  if(authority.lastIndexOf("@")>=0)secrets=authority.slice(0,authority.lastIndexOf("@")).split(":")
  var query=raw.indexOf("?")>=0 ? raw.slice(raw.indexOf("?")+1).split("#")[0] : ""
  query.split("&").forEach(function(pair){var n=pair.indexOf("=");if(n>=0)secrets.push(pair.slice(n+1).replace(/\+/g," "))})
  if(raw.indexOf("#")>=0)secrets.push(raw.slice(raw.indexOf("#")+1))
  secrets=secrets.concat(secrets.map(function(s){try{return decodeURIComponent(s)}catch(error){return s}}))
  text=text.replace(/[a-z][a-z0-9+.-]*:\/\/[^\s"'<>]+/gi,publicUrl)
  secrets.sort(function(a,b){return b.length-a.length}).forEach(function(secret){if(secret)text=text.split(secret).join("…")})
  return text.slice(0,200)
}
function status(value) {return {ok:"OK",error:"Error",updating:"Updating",not_contacted:"Not contacted"}[value] || "—"}
