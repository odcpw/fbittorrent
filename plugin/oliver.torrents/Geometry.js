.pragma library

// Measured layout, for capture comparisons and the read-only widget state hook.
function card(item) {
  while(item && item.parent) {
    if(item.borderSpec!==undefined && item.padding!==undefined)break
    item=item.parent
  }
  return item
}
function describe(item,titleSize) {
  item=card(item)
  if (!item || item.width<=0 || item.height<=0)return null
  var result={width:item.width,height:item.height,titleBaseline:null,metaBaseline:null,separatorY:null,rowPitch:null}
  function walk(node) {
    if(node.visible && String(node).indexOf("PanelSeparator")>=0 && result.separatorY===null)result.separatorY=node.mapToItem(item,0,0).y
    if(node.torrent!==undefined && node.torrent && result.rowPitch===null)result.rowPitch=node.height+(node.parent && node.parent.spacing!==undefined ? node.parent.spacing : 0)
    if(node.text!==undefined && node.baselineOffset!==undefined && node.visible) {
      var p=node.mapToItem(item,0,node.baselineOffset)
      if(result.titleBaseline===null && node.font && node.font.pixelSize===titleSize && node.font.bold)result.titleBaseline=p.y
      else if(result.metaBaseline===null && (String(node.text).indexOf("↓")>=0 || /OFFLINE|CONNECTED/.test(String(node.text))))result.metaBaseline=p.y
    }
    if(node.children)for(var i=0;i<node.children.length;i++)walk(node.children[i])
  }
  walk(item)
  return result
}
