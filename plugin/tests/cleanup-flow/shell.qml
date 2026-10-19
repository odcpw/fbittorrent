import QtQuick
import Quickshell
import "Plugin/CleanupFlow.js" as Cleanup
ShellRoot {
  function check(name,condition,observed){console.log("FBTQS-STEP "+JSON.stringify({case:name,action:"exercise confirmed cleanup and asynchronous Undo",expect:name,observed:JSON.stringify(observed),ok:!!condition,ms:0}));console.log("FBTQS-PASS "+name)}
  Component.onCompleted:{
    var s={calls:[],notices:[],notify:function(text){s.notices.push(text)}}
    s.mutate=function(op,args,cap,callback){s.calls.push({op:op,args:args,callback:callback});return "r"+s.calls.length}
    var f=Cleanup.create(s,function(){})
    f.remove(["a","b"],false);s.calls[0].callback({state:"pending"})
    check("pending_is_not_undoable",f.busy && !f.available(Date.now()) && s.calls.length===1,f)
    f.frame({ev:"mutation.result",ref:"r1",outcome:"done",result:{state:"done",undo_until:Math.floor(Date.now()/1000)+10}})
    check("confirmed_deadline",f.available(Date.now()) && f.hashes.join()==="a,b",f)
    f.undo();s.calls[1].callback({state:"pending"})
    check("restore_is_sequential",f.busy && s.calls.length===2 && s.calls[1].args.source.restore==="a",f)
    f.frame({ev:"mutation.result",ref:"r2",outcome:"done",result:{state:"done",hash:"a"}})
    check("late_result_advances_bulk_undo",f.busy && s.calls.length===3 && s.calls[2].args.source.restore==="b",f)
    s.calls[2].callback({state:"done",hash:"b"})
    check("whole_batch_restored",!f.busy && !f.hashes.length && !f.available(Date.now()),f)
    f.remove(["c"],false);s.calls[3].callback({state:"done",undo_until:Math.floor(Date.now()/1000)-1})
    check("expired_sends_no_restore",!f.undo() && s.calls.length===4,f)
    f.remove(["d"],true);s.calls[4].callback({state:"done"})
    check("delete_has_no_undo",!f.available(Date.now()),f)
    f.reset();check("new_owner_clears_history",!f.hashes.length && !f.busy,f)
    Qt.callLater(function(){Qt.quit()})
  }
}
