.pragma library

function channels(color) {
  if (typeof color==="string" && /^#[0-9a-f]{6}$/i.test(color)) return [parseInt(color.slice(1,3),16),parseInt(color.slice(3,5),16),parseInt(color.slice(5,7),16)]
  return [Math.round(color.r*255),Math.round(color.g*255),Math.round(color.b*255)]
}
function hex(values) {return "#"+values.map(function(v){return Math.max(0,Math.min(255,Math.round(v))).toString(16).padStart(2,"0")}).join("")}
function luminance(color) {
  var v=channels(color).map(function(c){c/=255;return c<=0.04045 ? c/12.92 : Math.pow((c+0.055)/1.055,2.4)})
  return 0.2126*v[0]+0.7152*v[1]+0.0722*v[2]
}
function contrast(first,second) {var a=luminance(first),b=luminance(second);return (Math.max(a,b)+0.05)/(Math.min(a,b)+0.05)}
function mix(fg,bg,weight) {var a=channels(fg),b=channels(bg);return hex(a.map(function(v,i){return weight*v+(1-weight)*b[i]}))}
function dim(fg,bg) {
  var rungs=[Qt.darker(fg,1.4),mix(fg,bg,0.75),mix(fg,bg,0.85),fg]
  for (var i=0;i<rungs.length;i++) if (contrast(rungs[i],bg)>=4.5) return hex(channels(rungs[i]))
  return hex(channels(fg))
}
function accentText(accent,bg,fg) {return {color:contrast(accent,bg)>=4.5 ? accent : fg,bold:contrast(accent,bg)<4.5}}
