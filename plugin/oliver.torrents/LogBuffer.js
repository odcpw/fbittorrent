.pragma library
// Append-only, sequence-ordered ring. Compaction is amortized, not per entry.
function create(){
 var b={values:Object.create(null),version:0,delta:{added:[],removed:[]}},order=[],head=0,cached=[],cachedVersion=-1,names=Object.create(null),sourceNames=[],sourcesDirty=true
 b.append=function(entries){
  var added=[],removed=[],unordered=false,last=order.length ? Number(order[order.length-1]) : -1
  entries.forEach(function(row){var key=String(row.seq);if(b.values[key])return;b.values[key]=row;order.push(key);added.push(row);if(row.seq<last)unordered=true;last=Math.max(last,row.seq);if(!names[row.source])sourcesDirty=true;names[row.source]=(names[row.source] || 0)+1})
  if(!added.length)return false
  if(unordered){order=order.slice(head).sort(function(a,b){return Number(a)-Number(b)});head=0}
  while(order.length-head>1000){var key=order[head++],row=b.values[key];delete b.values[key];if(!--names[row.source]){delete names[row.source];sourcesDirty=true};removed.push(Number(key))}
  if(head>=1000){order=order.slice(head);head=0}
  b.version++;b.delta={added:added.filter(function(row){return !!b.values[String(row.seq)]}),removed:removed};return true
 }
 b.rows=function(){if(cachedVersion!==b.version){cached=order.slice(head).map(function(key){return b.values[key]});cachedVersion=b.version}return cached}
 b.sources=function(){if(sourcesDirty){sourceNames=Object.keys(names).sort().slice(0,12);sourcesDirty=false}return sourceNames}
 return b
}
