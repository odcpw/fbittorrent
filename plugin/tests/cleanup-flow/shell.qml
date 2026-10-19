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
    check("expired_receipt_is_truthful",s.notices.slice(-1)[0].indexOf("Undo expired")>=0 && !f.hashes.length,s.notices)
    f.remove(["d"],true);s.calls[4].callback({state:"done"})
    check("delete_has_no_undo",!f.available(Date.now()),f)
    f.reset();check("new_owner_clears_history",!f.hashes.length && !f.busy,f)
    f.remove(["late"],false);s.calls[5].callback(null,{code:"timeout",message:"no answer"})
    check("uncertain_remove_never_replays",!f.busy && !f.available(Date.now()) && s.calls.length===6,f)
    f.frame({ev:"delta",topic:"events",added:[{ev:"mutation.result",ref:"r6",outcome:"done",result:{state:"done",undo_until:Math.floor(Date.now()/1000)+10}}]})
    check("late_success_restores_undo",f.available(Date.now()) && f.hashes.join()==="late" && s.calls.length===6,f)
    f.undo();s.calls[6].callback({state:"pending"})
    f.frame({ev:"mutation.result",ref:"r7",outcome:"done",result:{state:"done",hash:"late"}})
    f.remove(["late"],false);s.calls[7].callback({state:"pending"})
    check("remove_after_undo_stays_pending",f.busy && !f.available(Date.now()),f)
    f.frame({ev:"mutation.result",ref:"r8",outcome:"done",result:{state:"done",undo_until:Math.floor(Date.now()/1000)+10}})
    check("remove_after_undo_confirmed",!f.busy && f.available(Date.now()) && f.hashes.join()==="late",f)
    f.remove(["older"],false);s.calls[8].callback(null,{code:"timeout",message:"no answer"})
    f.remove(["newer"],false);s.calls[9].callback({state:"pending"})
    f.frame({ev:"mutation.result",ref:"r9",outcome:"done",result:{state:"done",undo_until:Math.floor(Date.now()/1000)+10}})
    check("old_receipt_preserves_new_action",f.busy && f.hashes.indexOf("older")<0,f)
    f.frame({ev:"mutation.result",ref:"r10",outcome:"done",result:{state:"done",undo_until:Math.floor(Date.now()/1000)+10}})
    check("newest_confirmed_undo_wins",!f.busy && f.hashes.join()==="newer",f)
    Qt.callLater(function(){Qt.quit()})
  }
}
