import QtQuick
import qs.Ui as Ui
import qs.Commons as Commons
import "../Palette.js" as Palette

Ui.BorderSurface {
  id:root
  property string title:""
  property bool focused:false
  property bool rail:false
  property color foreground:Commons.Color.foreground
  borderSpec:rail ? Commons.Border.none() : Commons.Border.controlSpec("normal",foreground,Commons.Color.accent)
  padding:Math.max(0,Commons.Style.spacing.lg-Commons.Border.left(borderSpec))
  color:rail ? Palette.mix(foreground,Commons.Color.background,0.06) : Commons.Color.background
  radius:rail ? 0 : Commons.Style.cornerRadius
  Rectangle {
    visible:root.rail;anchors.right:parent.right;anchors.top:parent.top;anchors.bottom:parent.bottom
    width:Commons.Style.space(1);color:Palette.mix(root.foreground,Commons.Color.background,0.27)
  }
  default property alias content:body.data
  Item {
    id:body
    anchors.fill:parent
    anchors.leftMargin:root.contentLeftInset
    anchors.rightMargin:root.contentRightInset
    anchors.topMargin:root.contentTopInset
    anchors.bottomMargin:root.contentBottomInset
  }
}
