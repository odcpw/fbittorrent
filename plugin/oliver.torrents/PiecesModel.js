.pragma library
var POPCOUNT=(function(){var out=[];for(var i=0;i<256;i++){var n=0;for(var b=i;b;b>>=1)n+=b&1;out.push(n)}return out})()
function decode(data){
 if(!data)return null
 if(!Number.isInteger(data.count) || data.count<0 || data.count>6291456 || !Number.isInteger(data.piece_size) || data.piece_size<=0 || typeof data.have!=="string" || data.have.length>1048576 || !/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(data.have))throw Error("pieces exceeds frame budget")
 // Qt 6.11 array-buffer overload keeps binary bytes intact, without the
 // string overload's UTF-8 conversion. No boxed arrays per bit/byte.
 var ascii=new Uint8Array(data.have.length),alphabet="ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
 for(var a=0;a<data.have.length;a++)ascii[a]=data.have.charCodeAt(a)
 var bits=new Uint8Array(Qt.atob(ascii.buffer)),have=0
 if(bits.length!==Math.ceil(data.count/8) || (data.have.endsWith("==") && (alphabet.indexOf(data.have.charAt(data.have.length-3))&15)) || (!data.have.endsWith("==") && data.have.endsWith("=") && (alphabet.indexOf(data.have.charAt(data.have.length-2))&3)))throw Error("incorrect piece bitfield encoding/length")
 if(data.count%8 && (bits[bits.length-1]&((1<<(8-data.count%8))-1)))throw Error("nonzero piece padding")
 if(!Array.isArray(data.priority) || data.priority.some(function(p,i){return !Number.isInteger(p) || p<0 || p>=data.count || (i>0 && data.priority[i-1]>=p)}))throw Error("invalid piece priorities")
 if(data.playable_end!==undefined && (!Number.isInteger(data.playable_end) || data.playable_end<0 || data.playable_end>data.count))throw Error("invalid piece prefix")
 for(var i=0;i<bits.length;i++)have+=POPCOUNT[bits[i]]
 return {data:data,bits:bits,priority:data.priority.slice(),have:have}
}
function has(map,index){return !!(map.bits[Math.floor(index/8)]&(128>>(index%8)))}
function allHave(map,first,end){
 while(first<end && first%8){if(!has(map,first++))return false}
 var byteEnd=Math.floor(end/8)
 for(var byte=Math.floor(first/8);byte<byteEnd;byte++)if(map.bits[byte]!==255)return false
 first=Math.max(first,byteEnd*8)
 while(first<end)if(!has(map,first++))return false
 return true
}
function cells(map,width){
 if(!map || !map.data.count || !width)return []
 var count=Math.min(map.data.count,width),out=[],priority=0
 for(var cell=0;cell<count;cell++){
  var first=Math.floor(cell*map.data.count/count),end=Math.floor((cell+1)*map.data.count/count)
  while(priority<map.priority.length && map.priority[priority]<first)priority++
  out.push({first:first,end:end,have:allHave(map,first,end),priority:priority<map.priority.length && map.priority[priority]<end,prefix:map.data.playable_end!==undefined && first<map.data.playable_end})
 }
 return out
}

// A missing live bitfield does not imply a check has been scheduled.
function unavailable(torrent){
 if(torrent && torrent.state==="done" && !torrent.checked)return "Completed data is parked; the piece map is not retained. Recheck to inspect verified pieces."
 if(torrent && torrent.state==="checking")return "Checking data… verified pieces will appear when it completes."
 if(torrent && torrent.state==="metadata")return "Waiting for metadata before pieces can be inspected."
 if(torrent && torrent.state==="waiting_vpn")return "Verified piece map is unavailable while the engine waits for the network."
 return "Waiting for a verified piece map from the daemon."
}
