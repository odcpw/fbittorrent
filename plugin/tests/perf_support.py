"""Measurement-only counters/aliases in a disposable plugin copy."""
import shutil

def instrument(source,destination):
    shutil.copytree(source,destination)
    path=destination/'Dashboard.qml'
    text=path.read_text().replace('  id:root\n','  id:root\n  readonly property var perfWindow:window\n  readonly property var perfReview:addSheet.item\n  property var perfPublish:({calls:0,ms:0})\n',1)
    signature='  function publishNow(inspect) {'
    assert text.count(signature)==1
    text=text.replace(signature,'  function publishNow(inspect) {if(disposing)return;var at=Date.now();try{return publishMeasured(inspect)}finally{perfPublish.calls++;perfPublish.ms+=Date.now()-at}}\n  function publishMeasured(inspect) {',1)
    path.write_text(text)
    path=destination/'Service.qml'
    text=path.read_text().replace('  id: root\n','  id: root\n  property int perfSyncRows:0\n  property int perfFrames:0\n',1)
    text=text.replace('  function syncRows() {','  function syncRows() {\n    perfSyncRows++')
    text=text.replace('  function applyFrame(frame, request) {','  function applyFrame(frame, request) {\n    perfFrames++')
    path.write_text(text)
