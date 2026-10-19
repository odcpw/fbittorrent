.pragma library
function base32(value) {
  var alphabet="ABCDEFGHIJKLMNOPQRSTUVWXYZ234567",bits=0,buffer=0,hex=""
  for(var i=0;i<value.length;i++){
    var digit=alphabet.indexOf(value[i].toUpperCase());if(digit<0)return ""
    buffer=(buffer<<5)|digit;bits+=5
    if(bits>=8){bits-=8;hex+=((buffer>>>bits)&255).toString(16).padStart(2,"0")}
  }
  return hex
}
function parse(input) {
  if(typeof input!=="string" || input.length>65536)return null
  var value=input.trim(),match
  if(/^file:\/\//i.test(value)) {
    match=/^file:\/\/(?:localhost)?(\/[^?#]*)$/i.exec(value)
    if(!match)return null
    try{value=decodeURIComponent(match[1])}catch(error){return null}
  }
  if(/[\x00-\x1f\x7f]/.test(value))return null
  if(/^magnet:\?/i.test(value)) {
    var hash="",parts=value.slice(value.indexOf("?")+1).split("#")[0].split("&")
    for(var i=0;i<parts.length;i++){
      var equal=parts[i].indexOf("="),key,parameter
      if(equal<0)continue
      try{key=decodeURIComponent(parts[i].slice(0,equal));parameter=decodeURIComponent(parts[i].slice(equal+1))}catch(error){return null}
      if(key.toLowerCase()!=="xt")continue
      match=/^urn:btih:([0-9a-f]{40}|[a-z2-7]{32})$/i.exec(parameter)
      if(!match)continue
      var digest=match[1].length===40 ? match[1].toLowerCase() : base32(match[1])
      if(hash && hash!==digest)return null
      hash=digest
    }
    return hash ? {source:{magnet:value},hash:hash} : null
  }
  if(/^https?:\/\/[^\s/?#]+(?:[/?#]|$)/i.test(value))return {source:{url:value},hash:""}
  if(value[0]==="/" && /\.torrent$/i.test(value))return {source:{path:value},hash:""}
  return null
}
function normalize(source) {
  if(!source || typeof source!=="object" || Array.isArray(source))return null
  var keys=Object.keys(source)
  if(keys.length!==1 || ["magnet","url","path"].indexOf(keys[0])<0)return null
  var parsed=parse(source[keys[0]])
  return parsed && parsed.source[keys[0]]!==undefined ? parsed : null
}
function drop(urls,text) {return urls && urls.length===1 ? parse(String(urls[0])) : (!urls || !urls.length) ? parse(text) : null}
