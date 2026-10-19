import QtQuick
import qs.Ui as Ui
import qs.Commons as Commons

Ui.BorderSurface {
  id:root
  property string title:""
  property bool focused:false
  property color foreground:Commons.Color.foreground
  borderSpec:Commons.Border.controlSpec("normal",foreground,Commons.Color.accent)
  padding:Math.max(0,Commons.Style.spacing.lg-Commons.Border.left(borderSpec))
  color:Commons.Color.background
  radius:Commons.Style.cornerRadius
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
