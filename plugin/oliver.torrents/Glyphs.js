.pragma library

// Names and cmap entries verified in the installed JetBrainsMono Nerd Font.
var codepoints = {download:0xF01DA, upload:0xF0552, paused:0xF03E4, done:0xF012C,
  vpn:0xF099D, error:0xF0026, add:0xF0415, power:0xF0425, refresh:0xF0450,
  terminal:0xF018D, play:0xF040A, folder:0xF0770, trash:0xF01B4, turtle:0xF0883,
  metadata:0xF01D8, checking:0xF04E6, stalled:0xF051F, queued:0xF0150,
  ready:0xF040D, unread:0x25CF, metadataText:0x25CC}
function glyph(role) { return String.fromCodePoint(codepoints[role]) }
var download=glyph("download"), upload=glyph("upload"), paused=glyph("paused"), done=glyph("done")
var vpn=glyph("vpn"), error=glyph("error"), add=glyph("add"), power=glyph("power"), refresh=glyph("refresh")
var terminal=glyph("terminal"), play=glyph("play"), folder=glyph("folder"), trash=glyph("trash"), turtle=glyph("turtle")
var metadata=glyph("metadata"), checking=glyph("checking"), stalled=glyph("stalled"), queued=glyph("queued"), ready=glyph("ready")
