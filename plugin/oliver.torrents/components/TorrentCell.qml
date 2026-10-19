import QtQuick
import QtQuick.Layouts
import qs.Commons as Commons
import "../Model.js" as Model

Item {
  id:root
  required property var torrent
  required property var column
  property string pendingText:""
  property bool failed:false
  readonly property real renderedProgress:progressRule.value
  readonly property string renderedState:stateText.text
  readonly property bool unknown:value()==="—"
  function value() {
    var t=torrent
    switch(column.name) {
      case "glyph":return failed ? "!" : pendingText || Model.glyph(t)
      case "state":return Model.stateLabel(t)
      case "name":return t.name
      case "size":return Model.size(t.size,true)
      case "down":return Model.rate(t.dl_bps,true)
      case "up":return Model.rate(t.ul_bps,true)
      case "eta":return Model.eta(t.eta_s,true)
      case "peers":return Model.count(t.peers)
      case "ratio":return t.ratio===null || t.ratio===undefined ? "—" : Number(t.ratio).toFixed(2)
      case "added":return Model.when(t.added_at).replace(/ today$/,"")
    }
    return ""
  }
  Text {
    id:label
    anchors.fill:parent
    anchors.leftMargin:root.column.name==="glyph" ? Commons.Style.space(3) : Commons.Style.spacing.controlPaddingX
    anchors.rightMargin:root.column.name==="glyph" ? Commons.Style.space(3) : Commons.Style.spacing.controlPaddingX
    visible:root.column.name!=="progress" && root.column.name!=="state"
    textFormat:Text.PlainText
    text:root.column.name==="name" && root.pendingText ? root.torrent.name+" · "+root.pendingText : root.value()
    color:root.failed && root.column.name==="glyph" ? Commons.Color.urgent : root.column.name==="state" && root.torrent.state==="error" ? Commons.Color.urgent : root.unknown || (!!root.pendingText && root.column.name==="name") ? Model.dim(Commons.Color.foreground,Commons.Color.background) : Commons.Color.foreground
    font.family:Commons.Style.font.family
    font.pixelSize:Commons.Style.font.subtitle
    font.italic:!!root.pendingText && root.column.name==="name"
    verticalAlignment:Text.AlignVCenter
    horizontalAlignment:root.column.name==="glyph" ? Text.AlignHCenter : root.column.name==="name" ? Text.AlignLeft : Text.AlignRight
    elide:Text.ElideRight
  }
  StateText {
    id:stateText
    visible:root.column.name==="state"
    anchors.fill:parent
    anchors.leftMargin:Commons.Style.spacing.controlPaddingX
    anchors.rightMargin:Commons.Style.spacing.controlPaddingX
    verticalAlignment:Text.AlignVCenter
    torrent:root.torrent
  }
  ProgressRule {
    id:progressRule
    visible:root.column.name==="progress"
    anchors.fill:parent
    anchors.leftMargin:Commons.Style.spacing.controlPaddingX
    anchors.rightMargin:Commons.Style.spacing.controlPaddingX
    value:root.torrent.progress
    complete:root.torrent.state==="done" || root.torrent.state==="seeding"
  }
  HoverHandler {id:hover}
  Loader {
    anchors.fill:parent
    active:hover.hovered && root.column.name==="name"
    sourceComponent:FullTextTip {
      parent:root
      visible:true
      text:root.torrent.name+"\n"+Model.stateLabel(root.torrent)+" · "+Model.size(root.torrent.size,false)+"\n↓ "+Model.rate(root.torrent.dl_bps,false)+" · ↑ "+Model.rate(root.torrent.ul_bps,false)+" · "+Model.eta(root.torrent.eta_s,false)+"\nPeers "+Model.count(root.torrent.peers)+" · Ratio "+(root.torrent.ratio===undefined || root.torrent.ratio===null ? "—" : Number(root.torrent.ratio).toFixed(2))+"\nAdded "+Model.when(root.torrent.added_at)
    }
  }
}
