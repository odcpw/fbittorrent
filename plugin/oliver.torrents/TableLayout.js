.pragma library

var specs=[['glyph','',26],['state','STATE',110],['name','NAME',0],['size','SIZE',76],['progress','PROGRESS',120],['down','↓',80],['up','↑',80],['eta','ETA',64],['peers','PEERS',56],['ratio','RATIO',56],['added','ADDED',80]]
function tier(width) {return width>=1600 ? 3 : width>=1400 ? 2 : width>=1100 ? 1 : 0}
function chrome(width) {return tier(width)>=2 ? 636 : tier(width)===1 ? 246 : 16}
function columns(windowWidth,contentWidth,scale) {
  var full=windowWidth>=1600,hidden=full ? [] : ['up','ratio','added']
  var fixed=0
  var result=specs.map(function(spec){
    var hide=hidden.indexOf(spec[0])>=0,width=hide ? 0 : Math.round(spec[2]*scale)
    fixed+=width
    return {name:spec[0],label:spec[1],width:width,hidden:hide}
  })
  result[2].width=Math.max(0,contentWidth-fixed)
  return result
}
function descending(key){return ["added","progress","rate","size","up","peers","ratio"].indexOf(key)>=0}
function sortKey(column) {return {down:'rate',added:'added',name:'name',state:'state',progress:'progress',size:'size',up:'up',eta:'eta',peers:'peers',ratio:'ratio'}[column] || null}
