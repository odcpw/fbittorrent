import QtQuick
import qs.Commons as Commons
import qs.Ui as Ui
import "../BarState.js" as BarState
import "../Glyphs.js" as Glyphs
import "../Model.js" as Model

Ui.BarIconButton {
  id: root
  required property var service
  property bool showRates: false
  readonly property var appearance: BarState.describe(service)
  text: appearance.glyph
  labelText: showRates ? appearance.rate : ""
  tooltipText: appearance.tooltip
  FontMetrics { id:labelMetrics; font.family:root.fontFamily; font.pixelSize:root.fontSize }
  fixedWidth: vertical ? -1 : Math.max(slotSize,canvasSize+(labelText ? labelMetrics.advanceWidth(labelText)+Commons.Style.space(4) : 0))
  foreground: appearance.tone==="urgent" ? (bar ? bar.urgent : Commons.Color.urgent)
    : appearance.tone==="normal" ? (bar ? bar.barForeground : Commons.Color.foreground)
    : Model.dim(bar ? bar.barForeground : Commons.Color.foreground,bar && bar.background ? bar.background : Commons.Color.background)
  Behavior on foreground {
    enabled: !!root.bar && root.bar.foregroundAnimationEnabled
    ColorAnimation { duration: Commons.Style.duration(160) }
  }
}
