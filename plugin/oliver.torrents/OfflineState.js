.pragma library
.import "Store.js" as Store

// Observations belong to a live connection. Never present its last snapshot
// as current activity after bye/EOF, even if the same daemon reconnects.
function clear(service) {
  service.store=Store.create()
  service.orderBoundary(true)
  service.torrentModel.clear()
  service.torrents=[]
  service.orderBoundary(false)
  service.badgeCounts={}
  service.topicVersions={}
  service.clipboardOffer=null
  service.observedLogVersion=0
  service.logRevision++
  service.statsRevision++
  if(service.detailFlow)service.detailFlow.hide()
  if(service.addFlow)service.addFlow.disconnected()
}
