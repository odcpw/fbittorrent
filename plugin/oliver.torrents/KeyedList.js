.pragma library
// Preserve native delegates and scroll positions while keyed observations change.
function create(){
 var previous=[]
 return {sync:function(model,rows,key){
  var wanted=Object.create(null);rows.forEach(function(row){wanted[String(row[key])]=true})
  for(var i=previous.length-1;i>=0;i--)if(!wanted[String(previous[i][key])]){previous.splice(i,1);model.remove(i)}
  rows.forEach(function(row,index){
   var id=String(row[key]),at=index
   while(at<previous.length && String(previous[at][key])!==id)at++
   if(at===previous.length){previous.splice(index,0,row);model.insert(index,{entryKey:id,entry:row})}
   else {if(at!==index){model.move(at,index,1);previous.splice(index,0,previous.splice(at,1)[0])}if(previous[index]!==row){previous[index]=row;model.setProperty(index,"entry",row)}}
  })
 }}
}
