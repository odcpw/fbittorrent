.pragma library
// Bounded read queue; QML receives only daemon-owned local poster paths.
function create(service,changed){
 var cache=Object.create(null),order=[],jobs=Object.create(null),queue=[],active=false,pending=0
 var p={}
 function keep(id,card){if(!Object.prototype.hasOwnProperty.call(cache,id)){order.push(id);if(order.length>128)delete cache[order.shift()]}cache[id]=card}
 function finish(id,result,failure){
  if(!jobs[id])return
  delete jobs[id];pending--
  if(!failure)keep(id,result ? result.card : null)
  changed();pump()
 }
 function pump(){
  while(active && pending<4 && queue.length){
   var id=queue.shift()
   if(jobs[id] || Object.prototype.hasOwnProperty.call(cache,id))continue
   jobs[id]=true;pending++
   dispatch(id)
  }
 }
 function dispatch(id){var sent=service.lookupImdb({id:id},function(result,failure){finish(id,result,failure)});if(sent===false)finish(id,null,{message:"IMDb unavailable"})}
 p.load=function(rows){
  active=true;queue=[];var wanted=Object.create(null)
  rows.slice(0,60).forEach(function(row){var id=row.imdb_id;if(!id || wanted[id] || jobs[id] || Object.prototype.hasOwnProperty.call(cache,id))return;wanted[id]=true;queue.push(id)})
  pump();changed()
 }
 p.hide=function(){active=false;queue=[]}
 p.card=function(id){return cache[id] || null}
 p.state=function(){return {queued:queue.length,pending:pending,cached:order.length}}
 return p
}
