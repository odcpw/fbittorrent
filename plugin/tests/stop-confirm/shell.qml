import QtQuick
import Quickshell
import "Plugin"
import "Plugin/Model.js" as Model
import "Plugin/Store.js" as Store

ShellRoot {
  Service { id: service }
  function check(name, status, expected) {
    service.status=status
    var ok=service.needsStopConfirm()===expected
    console.log("FBTQS-STEP "+JSON.stringify({case:name,action:"evaluate the real Service stop rule",expect:String(expected),observed:JSON.stringify(status),ok:ok,ms:0}))
    console.log((ok ? "FBTQS-PASS " : "FBTQS-FAIL ")+name)
  }
  Component.onCompleted: {
    check("hello_before_summary",Model.apiStatus(Store.create(),true),true)
    check("initial_status",{ok:true},true)
    var store=Store.create()
    store.summary={counts:{total:0}}
    check("confirmed_idle",Model.apiStatus(store,true),false)
    for(var state of ["checking","stalled","downloading","seeding"]) {
      var counts={total:1};counts[state]=1;store.summary={counts:counts}
      check(state,Model.apiStatus(store,true),true)
    }
    store.summary={counts:{total:1,done:1},ul_bps:1}
    check("payload_still_flowing",Model.apiStatus(store,true),true)
    Qt.callLater(function(){Qt.quit()})
  }
}
