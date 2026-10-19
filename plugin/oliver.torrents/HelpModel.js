.pragma library
function groups(actions){
 var result=[],byScope={}
 actions.forEach(function(action){
  if(!Array.isArray(action.keys.dashboard))return
  var group=byScope[action.scope]
  if(!group){group={scope:action.scope,rows:[]};byScope[action.scope]=group;result.push(group)}
  group.rows.push(action)
 })
 return result
}
