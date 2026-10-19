import QtQuick
import Quickshell
import "Plugin/RssFlow.js" as Rss
ShellRoot {
  function check(name,condition,observed){console.log("FBTQS-STEP "+JSON.stringify({case:name,action:"exercise RSS workflow",expect:name,observed:JSON.stringify(observed),ok:!!condition,ms:0}));if(!condition)throw Error(name);console.log("FBTQS-PASS "+name)}
  Component.onCompleted:{
    var s={online:true,rssTopics:[],store:{topics:{"rss.feeds":{f:{id:"f",name:"Feed",unread:2}}},mutations:{}},calls:[],reviews:[]},changes=0
    s.client={request:function(op,args,options){s.calls.push({op:op,args:args,done:options.callback});return s.calls.length}}
    s.mutate=function(op,args,cap,callback){s.calls.push({op:op,args:args,done:callback});return "r"+s.calls.length}
    s.beginRssAdd=function(feed,item){s.reviews.push({feed:feed,item:item})}
    var f=Rss.create(s,function(){changes++});f.mount()
    check("read_only_mount",s.calls.every(function(c){return c.op==="rss.feeds" || c.op==="rss.items"}),s.calls)
    check("selected_feed_topic",s.rssTopics.join(",")==="rss.items:f",s.rssTopics)
    s.store.topics['rss.items:f']={a:{id:"a",title:"Article",read:false,grabbed:false},b:{id:"b",title:"Newest",published_at:100,read:false,grabbed:false}}
    f.frame({ev:"snapshot.end",topic:"rss.items:f"});check("unknown_size_stays_unknown",f.items[0].id==="b" && f.items[1].size===undefined,f.items)
    f.activate();check("feed_enter_only_opens_items",f.pane==="items" && s.reviews.length===0,s.reviews)
    f.activate();check("article_review_is_deliberate",s.reviews.length===1 && s.reviews[0].item.id==="b" && !s.store.topics['rss.items:f'].b.read,s.reviews)
    f.read(false);var call=s.calls[s.calls.length-1];check("read_toggle_uses_explicit_flag",call.op==="rss.set_read" && call.args.read===true && !s.store.topics['rss.items:f'].b.read,call.args)
    call.done({});f.read(true);check("mark_all_uses_feed_operation",s.calls[s.calls.length-1].op==="rss.mark_feed_read",s.calls)
    f.refresh(false);check("explicit_refresh_only",s.calls[s.calls.length-1].op==="rss.refresh",s.calls)
    f.removeOpen();f.cancel();check("cancel_does_not_remove",!s.calls.some(function(c){return c.op==="rss.remove"}),s.calls)
    var before=changes;f.frame({ev:"state",topic:"summary"});check("unrelated_updates_do_not_rebuild",changes===before,changes)
    f.read(false);call=s.calls[s.calls.length-1];call.done(undefined,{message:"Could not save read state"})
    check("failed_read_stays_unread",!s.store.topics['rss.items:f'].b.read && f.message==="Could not save read state",f.snapshot())
    f.read(false);call=s.calls[s.calls.length-1];var ref="r"+s.calls.length;call.done({state:"pending"})
    check("pending_read_has_no_optimistic_mark",!s.store.topics['rss.items:f'].b.read,s.calls)
    f.frame({ev:"mutation.result",ref:ref,outcome:"failed",error:{message:"Read rejected"}})
    check("async_failure_is_visible",f.message==="Read rejected" && !s.store.topics['rss.items:f'].b.read,f.snapshot())
    f.hide();check("hidden_releases_items",s.rssTopics.length===0 && !s.store.topics['rss.items:f'],s.rssTopics)
    f.mount();check("reopen_resubscribes",s.rssTopics[0]==="rss.items:f",s.rssTopics)
    Qt.callLater(function(){Qt.quit()})
  }
}
