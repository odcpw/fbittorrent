.pragma library
function create(service,changed) {
  var f={visible:false,feeds:[],items:[],feed:"",item:"",pane:"feeds",modal:"",message:"",loading:false,unread:false},epoch=0,pending={}
  function publish(){changed()}
  function topic(){return f.feed ? "rss.items:"+f.feed : ""}
  function release(){var old=service.rssTopics || [];service.rssTopics=[];old.forEach(function(t){delete service.store.topics[t]})}
  function failure(error){f.message=error ? error.message : "No answer; refresh to check the current state";publish()}
  function current(){return f.items.find(function(row){return row.id===f.item}) || null}
  function feed(){return f.feeds.find(function(row){return row.id===f.feed}) || null}
  function items(){
    var rows=service.store.topics[topic()] || {}
    f.items=Object.keys(rows).map(function(id){return rows[id]}).filter(function(row){return !f.unread || !row.read}).sort(function(a,b){return (b.published_at || 0)-(a.published_at || 0) || a.id.localeCompare(b.id)})
    if(!f.items.some(function(row){return row.id===f.item}))f.item=f.items.length ? f.items[0].id : ""
  }
  function feeds(){
    var rows=service.store.topics["rss.feeds"] || {}
    f.feeds=Object.keys(rows).map(function(id){return rows[id]}).sort(function(a,b){return a.name.localeCompare(b.name) || a.id.localeCompare(b.id)})
    if(!feed())f.chooseFeed(f.feeds.length ? f.feeds[0].id : "")
  }
  function refreshRead(){service.client.request("rss.feeds",{}, {quiet:true});if(f.feed)service.client.request("rss.items",{feed:f.feed},{quiet:true})}
  function send(op,args,done){
    if(Object.keys(pending).length>=8){failure({message:"Wait for the current RSS changes to finish"});return false}
    f.message="";var generation=epoch,ref=service.mutate(op,args,"rss",function(result,error){
      if(generation!==epoch || !f.visible)return
      if(error){delete pending[ref];failure(error);return}
      if(result.state==="pending")return
      delete pending[ref];if(done)done(result);refreshRead();publish()
    })
    if(ref)pending[ref]={generation:generation,done:done};else failure({message:service.lastError || "Request could not be sent"})
    return ref
  }
  f.mount=function(){epoch++;pending={};f.visible=true;f.pane="feeds";f.modal="";feeds();if(f.feed)f.chooseFeed(f.feed);refreshRead();publish()}
  f.hide=function(){epoch++;f.visible=false;pending={};release()}
  f.chooseFeed=function(id){
    if(id===f.feed && service.rssTopics.length)return
    release();f.feed=id;f.item="";f.items=[];f.loading=!!id
    if(id)service.rssTopics=[topic()]
    publish()
  }
  f.chooseItem=function(id){f.item=id;f.pane="items";publish()}
  f.move=function(delta,end){
    var rows=f.pane==="feeds" ? f.feeds : f.items,key=f.pane==="feeds" ? "feed" : "item"
    if(!rows.length)return
    var at=rows.findIndex(function(row){return row.id===f[key]});at=end===undefined ? Math.max(0,Math.min(rows.length-1,at+delta)) : end<0 ? rows.length-1 : 0
    if(key==="feed")f.chooseFeed(rows[at].id);else f.item=rows[at].id
    publish()
  }
  f.activate=function(){if(f.pane==="feeds"){f.pane="items";publish()}else {var row=current();if(row)service.beginRssAdd(f.feed,row)}}
  f.back=function(){f.pane="feeds";publish()}
  f.read=function(all){if(!all && f.pane!=="items"){f.message="Enter opens articles · M marks the feed read";publish();return}var row=current();if(all && feed())send("rss.mark_feed_read",{feed:f.feed});else if(row)send("rss.set_read",{feed:f.feed,item:row.id,read:!row.read})}
  f.refresh=function(all){send("rss.refresh",all ? {} : f.feed ? {feed:f.feed} : {})}
  f.filter=function(){f.unread=!f.unread;items();publish()}
  f.addOpen=function(){f.modal="add";f.message="";publish()}
  f.add=function(url,name){
    if(!/^https?:\/\//.test(url.trim())){failure({message:"Enter an HTTP or HTTPS feed URL"});return false}
    send("rss.add",{url:url.trim(),name:name.trim() || undefined},function(result){f.modal="";if(result.feed)f.chooseFeed(result.feed.id)});return true
  }
  f.removeOpen=function(){if(feed()){f.modal="remove";publish()}}
  f.remove=function(){if(feed())send("rss.remove",{feed:f.feed},function(){f.modal="";f.pane="feeds";f.chooseFeed("")})}
  f.cancel=function(){f.modal="";f.message="";publish()}
  f.current=current;f.currentFeed=feed
  f.frame=function(frame,request){
    if(!f.visible)return
    if(frame.ev==="mutation.result" && frame.outcome!=="pending" && pending[frame.ref]){
      var task=pending[frame.ref];delete pending[frame.ref]
      if(task.generation===epoch){if(frame.outcome==="done"){if(task.done)task.done(frame.result || {});refreshRead();publish()}else failure(frame.error)}
    }
    if(frame.topic==="events" && (frame.ev==="snapshot.end" || frame.ev==="delta"))Object.keys(pending).forEach(function(ref){var result=service.store.mutations[ref];if(result)f.frame({ev:"mutation.result",ref:ref,outcome:result.status,result:result.result,error:result.error})})
    var committed=frame.ev==="snapshot.end" || frame.ev==="delta" || (frame.ok && request)
    if(!committed || !(frame.topic==="rss.feeds" || frame.topic===topic() || (request && ["rss.feeds","rss.items"].indexOf(request.op)>=0)))return
    if(frame.topic==="rss.feeds" || (request && request.op==="rss.feeds"))feeds()
    if(frame.topic===topic() || (request && request.op==="rss.items" && request.args.feed===f.feed)){f.loading=false;items()}
    publish()
  }
  f.snapshot=function(){return {feeds:f.feeds,items:f.items,feed:f.feed,item:f.item,pane:f.pane,modal:f.modal,message:f.message,loading:f.loading,unread:f.unread}}
  return f
}
