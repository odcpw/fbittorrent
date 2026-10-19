.pragma library

function known(value) { return value !== null && value !== undefined && isFinite(Number(value)) && Number(value)>=0 }
function scaled(value, base, units, compact) {
  if (!known(value)) return "—"
  var n=Number(value), index=0
  while (n>=base && index<units.length-1) {n/=base;index++}
  return (index>0 && (!compact || n<10) ? n.toFixed(1) : n.toFixed(0))+(compact ? "" : " ")+units[index]
}
function size(value, compact) {return scaled(value,1024,compact ? ["B","K","M","G","T","P"] : ["B","KiB","MiB","GiB","TiB","PiB"],compact)}
function rate(value, compact) {return scaled(value,1000,compact ? ["B","K","M","G","T"] : ["B/s","KB/s","MB/s","GB/s","TB/s"],compact)}
function eta(value, compact) {
  if (!known(value)) return "—"
  var s=Math.floor(Number(value)), m=Math.floor(s/60), h=Math.floor(m/60), d=Math.floor(h/24)
  if (d>0) return d+(compact ? "d" : " d")
  if (h>0) return h+(compact ? "h" : " h ")+String(m%60).padStart(2,"0")
  if (m>0) return m+(compact ? "m" : " min")
  return s+(compact ? "s" : " s")
}
function percent(value, complete) {
  if (!known(value) || Number(value)>1) return "—"
  var n=Number(value)*100
  if (complete && n>=100) return "100%"
  if (n>0 && n<1) return "<1%"
  if (n<=99.5) return Math.min(99,Math.round(n))+"%"
  return Math.min(99.9,n).toFixed(1)+"%"
}
function count(value) {
  if (!known(value) || !Number.isInteger(Number(value))) return "—"
  var n=String(value)
  return Number(value)>9999 ? n.replace(/\B(?=(\d{3})+(?!\d))/g,"\u2009") : n
}
function date(value) {if (value===null || value===undefined) return null;var d=new Date(typeof value==="number" ? value*1000 : value);return isNaN(d.getTime()) ? null : d}
function when(value, nowValue) {
  var d=date(value), now=nowValue===undefined ? new Date() : date(nowValue)
  if (!d || !now) return "—"
  if (d.getFullYear()!==now.getFullYear()) return d.getFullYear()+"-"+String(d.getMonth()+1).padStart(2,"0")+"-"+String(d.getDate()).padStart(2,"0")
  if (d.getMonth()===now.getMonth() && d.getDate()===now.getDate()) return String(d.getHours()).padStart(2,"0")+":"+String(d.getMinutes()).padStart(2,"0")+" today"
  return ["Jan","Feb","Mar","Apr","May","Jun","Jul","Aug","Sep","Oct","Nov","Dec"][d.getMonth()]+" "+d.getDate()
}
function ago(value, nowValue) {
  var d=date(value), now=nowValue===undefined ? new Date() : date(nowValue)
  if (!d || !now) return "—"
  var seconds=Math.floor((now-d)/1000)
  if (seconds<0) return "in "+eta(-seconds,false)
  return eta(seconds,false)+" ago"
}
