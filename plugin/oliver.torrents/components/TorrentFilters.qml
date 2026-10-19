import QtQuick
import qs.Ui as Ui
import qs.Commons as Commons
import "../Model.js" as Model

Column {
  id:root
  required property var service
  property string selected:"all"
  property bool focused:false
  property bool chips:false
  property int cursor:0
  signal chosen(string key)
  signal routed(string view)
  readonly property bool vpn:!!(service.store.network.selection || service.store.settings.network_interface)
  readonly property var choices:[{key:"all",label:"All"},{key:"downloading",label:"Downloading"},{key:"ready",label:"Ready"},{key:"queued",label:"Queued"},{key:"paused",label:"Paused"},{key:"reviewing",label:"Reviewing"},{key:"done",label:"Done"},{key:"seeding",label:"Uploading"},{key:"error",label:"Errors"}].concat(vpn ? [{key:"waiting_vpn",label:"Waiting for VPN"}] : [])
  function count(key) {
    if(!service.status.loaded)return undefined
    var c=service.store.counts
    if(key==="all")return c.total || 0
    if(key==="downloading")return (c.downloading || 0)+(c.stalled || 0)+(c.metadata || 0)+(c.checking || 0)
    return c[key] || 0
  }
  function move(delta){cursor=Math.max(0,Math.min(choices.length-1,cursor+delta));chosen(choices[cursor].key)}
  spacing:Commons.Style.spacing.xxs
  Text {visible:!root.chips;text:"FILTERS";textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.bodySmall}
  Rectangle {visible:!root.chips && root.focused;width:parent.width;height:Commons.Style.space(2);color:Commons.Color.accent}
  Flow {
    width:parent.width
    spacing:root.chips ? Commons.Style.spacing.xs : 0
    Repeater {
      model:root.choices
      Ui.BorderSurface {
        required property int index
        required property var modelData
        width:root.chips ? caption.implicitWidth+Commons.Style.space(20) : root.width
        height:Commons.Style.spacing.popupRowHeight
        borderSpec:Commons.Border.none()
        color:root.selected===modelData.key ? Commons.Style.selectedFill : hovered.hovered && root.focused ? Commons.Style.selectedAccentFill : "transparent"
        radius:Commons.Style.cornerRadius
        Text {id:caption;anchors.verticalCenter:parent.verticalCenter;anchors.left:parent.left;anchors.leftMargin:Commons.Style.spacing.lg;textFormat:Text.PlainText;text:modelData.label+"  "+Model.count(root.count(modelData.key));color:root.count(modelData.key) ? Commons.Color.foreground : Model.dim(Commons.Color.foreground,Commons.Color.background);font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.subtitle}
        HoverHandler {id:hovered}
        MouseArea {anchors.fill:parent;acceptedButtons:Qt.LeftButton;onClicked:{root.cursor=index;root.chosen(modelData.key)}}
      }
    }
  }
  Ui.Button {visible:!root.chips;text:"Shows"+(root.service.badgeCounts && root.service.badgeCounts.shows>0 ? " ●"+root.service.badgeCounts.shows : "");onClicked:root.routed("shows")}
  Ui.Button {visible:!root.chips;text:"RSS"+(root.service.badgeCounts && root.service.badgeCounts.rss>0 ? " ●"+root.service.badgeCounts.rss : "");onClicked:root.routed("rss")}
}
