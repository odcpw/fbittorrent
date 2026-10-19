import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import qs.Commons
import qs.Commons as Commons
import qs.Ui
import qs.Ui as Ui
import "../Model.js" as Model

PanelActionButton {
  property var host

    property int index: 0
    foreground: host.foreground
    fontFamily: host.fontFamily
    bordered: true
    hasCursor: host.cursorActive && host.focusSection === "header" && host.headerIndex === index
    onHovered: function(on) { if (on) host.setHeaderCursor(index) }
  }
