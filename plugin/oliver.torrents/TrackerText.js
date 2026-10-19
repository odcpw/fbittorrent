.pragma library
function decoded(value) {
  value=String(value || "")
  for(var i=0;i<4;i++){try{var next=decodeURIComponent(value);if(next===value)break;value=next}catch(error){break}}
  return value
}
function parameter(value){return /^(passkey|key|token|api_key|apikey|authkey|password|secret)$/i.test(value)}
function parts(raw) {
  var path=String(raw || "").replace(/^[a-z][a-z0-9+.-]*:\/\/[^/?#]*/i,"").split(/[?#]/)[0],previous=false
  return path.split("/").map(function(part){
    var value=decoded(part).replace(/^[\[\](.,]+|[\[\]),.]+$/g,""),named=value.split("/").some(parameter) || value.split(/[;&/]/).some(function(item){var n=item.indexOf("=");return n>=0 && parameter(item.slice(0,n))})
    var secret=previous || (/passkey/i.test(value)&&!parameter(value)) || /^[a-z0-9+_\/=-]{16,}$/i.test(value) || /%[0-9a-f]{2}/i.test(value) || named
    previous=parameter(value.split("/").pop());return {value:part,secret:secret}
  })
}
function publicUrl(value) {
  value=String(value || "")
  if(!/^(https?|udp):\/\//i.test(value))return "Unsupported tracker URL"
  var authority=value.match(/^[a-z][a-z0-9+.-]*:\/\/[^/?#]*/i)[0].replace(/^([a-z][a-z0-9+.-]*:\/\/)[^/?#]*@/i,"$1")
  return authority+parts(value).map(function(part){return part.secret ? "REDACTED" : part.value}).join("/")
}
function message(value,url) {
  var text=String(value || "").replace(/[\x00-\x1f\x7f]/g," "),raw=String(url || ""),secrets=[]
  function remember(value){if(!value)return;secrets.push(value);secrets.push(decoded(value))}
  function collect(raw){
    var authority=(raw.match(/^[a-z][a-z0-9+.-]*:\/\/([^/?#]*)/i) || [])[1] || ""
    if(authority.lastIndexOf("@")>=0)authority.slice(0,authority.lastIndexOf("@")).split(":").forEach(remember)
    var query=raw.indexOf("?")>=0 ? raw.slice(raw.indexOf("?")+1).split("#")[0] : ""
    query.split("&").forEach(function(pair){var n=pair.indexOf("=");if(n>=0){remember(pair.slice(n+1));remember(pair.slice(n+1).replace(/\+/g," "))}else remember(pair)})
    if(raw.indexOf("#")>=0)remember(raw.slice(raw.indexOf("#")+1))
    parts(raw).forEach(function(part){if(part.secret){remember(part.value);remember(decoded(part.value).replace(/^[\[\](.,]+|[\[\]),.]+$/g,""));var layer=part.value;for(var i=0;i<5;i++){var previous=false;layer.split("/").forEach(function(segment){if(previous)remember(segment);previous=parameter(segment);segment.split(/[;&]/).forEach(function(pair){var n=pair.indexOf("=");if(n>=0 && parameter(pair.slice(0,n)))remember(pair.slice(n+1))});var n=segment.indexOf("=");if(n>=0 && parameter(segment.slice(0,n).split(/[;&]/).pop()))remember(segment.slice(n+1))});var next=decoded(layer);if(next===layer)break;layer=next}}})
  }
  collect(raw)
  text=text.replace(/[a-z][a-z0-9+.-]*:\/\/[^\s"'<>(),]+/gi,function(value){collect(value);return publicUrl(value)})
  text=decoded(text).replace(/[\x00-\x1f\x7f]/g," ")
  text=text.replace(/[a-z][a-z0-9+.-]*:\/\/[^\s"'<>(),]+/gi,function(value){collect(value);return publicUrl(value)})
  text=text.replace(/\b(passkey|key|token|api_key|apikey|authkey|password|secret)=(?:"([^"\n]*)"|'([^'\n]*)'|([^\s;&,"'\])}]+))/gi,function(full,key,double,single,plain){remember(double!==undefined ? double : single!==undefined ? single : plain);return full})
  secrets.sort(function(a,b){return b.length-a.length}).forEach(function(secret){if(secret)text=text.split(secret).join("…")})
  return text.slice(0,200)
}
function status(value) {return {ok:"OK",error:"Error",updating:"Updating",not_contacted:"Not contacted"}[value] || "—"}
