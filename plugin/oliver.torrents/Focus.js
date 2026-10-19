.pragma library

function command(address,usingLua) {
  if(typeof address!=="string" || !/^(?:0x)?[0-9a-f]+$/i.test(address))return null
  var selector="address:"+address
  return usingLua ? "hl.dsp.focus({window="+JSON.stringify(selector)+"})" : "focuswindow "+selector
}
