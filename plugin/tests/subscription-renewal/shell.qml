import QtQuick
import Quickshell
import "Plugin/ApiClient.js" as ApiClient
ShellRoot {
  function require(ok,message){if(!ok)throw Error(message)}
  Component.onCompleted:{
    var stage="coalesced_renewal"
    try {
    var clock=0,writes=[],client=ApiClient.create({write:function(line){writes.push(JSON.parse(line))},flush:function(){},isConnected:function(){return true},disconnect:function(){}},{now:function(){return clock}})
    client.visibility(false,true,[]);client.connected()
    client.receive(JSON.stringify({id:1,ok:true,result:{proto:1,instance:"a".repeat(32),caps:["torrents","subscribe","settings","shows","rss"]}}))
    for(var i=0;i<4;i++)client.visibility(false,true,["shows.releases:tt"+i])
    var active={},number=0,peak=0,busy=0,index=1
    function reply(frame,result,error){client.receive(JSON.stringify(error ? {id:frame.id,ok:false,error:error} : {id:frame.id,ok:true,result:result}))}
    for(;index<writes.length && index<200;index++){
      var frame=writes[index]
      if(!client.pending[frame.id])continue
      if(frame.op==="subscribe"){
        var count=Object.keys(active).reduce(function(n,key){return n+active[key].length},0)+frame.args.topics.length
        if(count>16){busy++;reply(frame,null,{code:"busy",message:"subscription topic budget",retryable:true});continue}
        peak=Math.max(peak,count);var sub=++number;active[sub]=frame.args.topics
        reply(frame,{sub:sub,topics:frame.args.topics})
        var rev=0
        frame.args.topics.forEach(function(topic){
          client.receive(JSON.stringify({ev:"snapshot.begin",sub:sub,rev:++rev,topic:topic,count:0}))
          client.receive(JSON.stringify({ev:"snapshot.end",sub:sub,rev:++rev,topic:topic}))
        })
      }else if(frame.op==="unsubscribe"){delete active[frame.args.sub];reply(frame,{state:"done"})}
      else reply(frame,{})
    }
    var required=["summary","torrents","events","settings","shows","rss.feeds","shows.releases:tt3"]
    require(index<200 && busy===0 && peak<=16,"renewal must stay within the server topic budget; busy="+busy+" peak="+peak)
    require(required.every(function(topic){return client.topicRevs[topic]>0}),"latest visibility retains every base/detail topic: "+JSON.stringify(client.topicRevs))
    require(Object.keys(active).length===2,"only latest base and detail groups survive: "+JSON.stringify(active))
    console.log("FBTQS-PASS coalesced_renewal")
    stage="transient_retry"
    // A transient rejection of the current group must not silently lose it.
    client.subscribeAll(true)
    while(index<writes.length){var old=writes[index++];if(old.op==="unsubscribe")reply(old,{state:"done"});else if(old.op==="subscribe")reply(old,null,{code:"busy",message:"transient budget",retryable:true});else reply(old,{})}
    var before=writes.length;clock=999;client.tick();require(writes.length===before,"retry respects its bounded delay")
    clock=1000;client.tick();require(writes.length>before && writes[writes.length-1].op==="subscribe","a rejected current base group is retried")
    var recovered=writes[writes.length-1];reply(recovered,{sub:101,topics:recovered.args.topics})
    var detail=writes[writes.length-1];reply(detail,{sub:102,topics:detail.args.topics})
    require(!client.lastError && client.subs[101] && client.subs[102],"successful renewal clears its transient failure and retains both groups")
    console.log("FBTQS-PASS transient_retry")
    }catch(error){console.log("FBTQS-FAIL "+stage+" "+String(error))}
    Qt.callLater(function(){Quickshell.execDetached(["qs","kill","-p",Quickshell.shellPath(".")])})
  }
}
