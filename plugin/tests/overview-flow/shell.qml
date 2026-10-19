import QtQuick
import Quickshell
import "Plugin/OverviewFlow.js" as Overview
ShellRoot {
  function check(name,condition,observed){console.log("FBTQS-STEP "+JSON.stringify({case:name,action:"exercise Overview actions",expect:name,observed:JSON.stringify(observed),ok:!!condition,ms:0}));if(!condition)throw Error(name);console.log("FBTQS-PASS "+name)}
  Component.onCompleted:{
    var s={calls:[],reads:[],lastError:"",refresh:function(){}}
    s.mutate=function(op,args,cap,callback){s.calls.push({op:op,args:args,callback:callback});return "r"+s.calls.length}
    s.client={request:function(op,args,options){s.reads.push({op:op,args:args,callback:options.callback})}}
    var f=Overview.create(s,function(){})
    check("cap_syntax",Overview.parseCap("64K")===65536 && Overview.parseCap("2M")===2097152 && Overview.parseCap("none")===null && Overview.parseCap("")===null,"K/M and unlimited")
    check("effective_cap_uses_active_global",Overview.effectiveCap({download_limit_bps:65536},{download_limit_bps:32768,alt_download_limit_bps:16384},false)===32768 && Overview.effectiveCap({download_limit_bps:65536},{download_limit_bps:32768,alt_download_limit_bps:16384},true)===16384 && Overview.effectiveCap({}, {max_active_downloads:1}, false)===null && Overview.effectiveCap({}, {}, false)===undefined,"minimum finite active caps")
    f.open("a","limit");check("invalid_cap_keeps_editor",!f.limit("1") && f.modal==="limit" && !s.calls.length,f.snapshot())
    f.limit("64K");check("cap_is_explicit_and_pending",s.calls[0].args.bps===65536 && f.busy("a") && !f.modal,s.calls[0].args)
    s.calls[0].callback({state:"done"});check("confirmed_cap_reads_current_row",!f.busy("a") && s.reads[0].op==="torrents.get",s.reads)
    f.open("a","move");f.cancel();check("cancel_sends_no_move",s.calls.length===1,f.snapshot())
    f.open("a","move");f.move("/new folder/#literal");s.calls[1].callback({state:"pending"});check("move_waits_for_its_result",f.busy("a") && !f.open("a","move"),s.calls[1].args)
    f.frame({ev:"mutation.result",ref:"r2",outcome:"failed",error:{code:"invalid_args",message:"Destination already exists"}})
    check("failed_move_preserves_server_message",!f.busy("a") && f.status("a")==="Destination already exists",f.status("a"))
    f.open("a","move");f.move("/new folder");s.calls[2].callback(null,{code:"timeout",message:"no answer"})
    check("uncertain_move_is_never_resent",f.busy("a") && !f.open("a","move") && s.calls.length===3,f.status("a"))
    f.refresh("a");s.reads[s.reads.length-1].callback({torrent:{hash:"a",save_path:"/old folder"}})
    check("old_location_keeps_move_unconfirmed",f.busy("a") && s.calls.length===3,f.status("a"))
    f.frame({ev:"delta",topic:"events",added:[{ev:"mutation.result",ref:"r3",outcome:"done",result:{state:"done"}}]})
    check("original_late_result_confirms_move",!f.busy("a") && f.status("a")==="Move complete",f.status("a"))
    f.open("a","move");f.move("/another folder");s.calls[3].callback(null,{code:"timeout",message:"no answer"});f.instanceChanged();f.refresh("a")
    s.reads[s.reads.length-1].callback({torrent:{hash:"a",save_path:"/confirmed previous folder"}})
    check("new_owner_read_resolves_old_request_without_resend",!f.busy("a") && s.calls.length===4 && f.status("a").indexOf("Confirmed location")>=0,f.status("a"))
    Qt.callLater(function(){Qt.quit()})
  }
}
