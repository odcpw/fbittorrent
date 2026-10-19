import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import qs.Commons
import qs.Commons as Commons
import qs.Ui
import qs.Ui as Ui
import "../Model.js" as Model

PanelToolTip {
    id: tip
    delay: 500
    property real panelWidth: parent ? parent.width : 0
    width: panelWidth
    x: 0
    y: parent ? parent.height + Style.space(6) : 0
    fontFamily: Style.font.family
    contentItem: Text {
      textFormat: Text.PlainText
      text: tip.text
      color: tip.panelForeground
      font.family: tip.fontFamily
      font.pixelSize: tip.fontSize
      wrapMode: Text.Wrap
      leftPadding: Border.left(tip.panelBorderSpec) + Style.spacing.controlPaddingX
      rightPadding: Border.right(tip.panelBorderSpec) + Style.spacing.controlPaddingX
      topPadding: Border.top(tip.panelBorderSpec) + Style.spacing.controlPaddingY
      bottomPadding: Border.bottom(tip.panelBorderSpec) + Style.spacing.controlPaddingY
    }
  }
