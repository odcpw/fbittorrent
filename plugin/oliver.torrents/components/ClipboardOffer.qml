import QtQuick
import QtQuick.Layouts
import qs.Commons as Commons
import qs.Ui as Ui

Column {
  id: root
  required property var service
  readonly property var offer: service ? service.clipboardOffer : null
  visible: !!offer
  spacing: Commons.Style.spacing.xxl
  onOfferChanged: if (offer) Qt.callLater(function(){addButton.forceActiveFocus()})
  Text {
    width:parent.width
    textFormat: Text.PlainText
    text: root.offer ? "Add '" + root.offer.name + "'?" : ""
    color: Commons.Color.foreground
    font.family: Commons.Style.font.family
    font.pixelSize: Commons.Style.font.body
    wrapMode: Text.Wrap
  }
  Row {
    spacing: Commons.Style.spacing.xxl
    Ui.Button { id:addButton; text:"Add"; focusable:true; bordered:true; onClicked:root.service.acceptOffer() }
    Ui.Button { text:"Cancel"; focusable:true; bordered:true; onClicked:root.service.cancelOffer() }
  }
}
