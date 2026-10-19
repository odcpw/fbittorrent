.pragma library
function select(entries,level,source,hash){return entries.filter(function(entry){return (!level || entry.level===level) && (!source || entry.source===source) && (!hash || entry.hash===hash)})}
function sources(entries){var names={};entries.forEach(function(entry){names[entry.source]=true});return Object.keys(names).sort().slice(0,12)}
