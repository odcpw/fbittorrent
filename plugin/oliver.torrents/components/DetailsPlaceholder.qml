import QtQuick
import qs.Commons as Commons
import "../Model.js" as Model

Column {
  id:root
  property var torrent:null
  property bool focused:false
  width:parent.width
  spacing:Commons.Style.spacing.panelGap
  Text {width:parent.width;textFormat:Text.PlainText;wrapMode:Text.Wrap;text:root.torrent ? root.torrent.name : "Details";color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.title;font.bold:true}
  Rectangle {visible:root.focused;width:parent.width;height:Commons.Style.space(2);color:Commons.Color.accent}
  Text {width:parent.width;textFormat:Text.PlainText;wrapMode:Text.Wrap;text:root.torrent ? Model.stateLabel(root.torrent)+" · "+Model.size(root.torrent.size,false)+"\n↓ "+Model.rate(root.torrent.dl_bps,false)+" · ↑ "+Model.rate(root.torrent.ul_bps,false)+"\nETA "+Model.eta(root.torrent.eta_s,false)+"\nPeers "+Model.count(root.torrent.peers)+" · Ratio "+(root.torrent.ratio===undefined || root.torrent.ratio===null ? "—" : Number(root.torrent.ratio).toFixed(2))+"\nAdded "+Model.when(root.torrent.added_at)+"\n"+root.torrent.save_path : "Select a torrent and press Enter.";color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
  KeyHint {width:parent.width;text:"Esc back · o open"}
}
