import QtQuick
import qs.Commons as Commons
import qs.Ui as Ui
import "../BarState.js" as BarState
import "../Glyphs.js" as Glyphs

Ui.BarIconButton {
  id: root
  required property var service
  property bool showRates: false
  readonly property var appearance: BarState.describe(service)
  text: appearance.glyph
  labelText: (appearance.turtle ? Glyphs.turtle : "") + (showRates && appearance.rate ? (appearance.turtle ? " " : "") + appearance.rate : "")
  tooltipText: appearance.tooltip
  FontMetrics { id:labelMetrics; font.family:root.fontFamily; font.pixelSize:root.fontSize }
  fixedWidth: vertical ? -1 : Math.max(slotSize,canvasSize+(labelText ? labelMetrics.advanceWidth(labelText)+Commons.Style.space(4) : 0))
  foreground: appearance.tone==="urgent" ? (bar ? bar.urgent : Commons.Color.urgent)
    : appearance.tone==="normal" ? (bar ? bar.barForeground : Commons.Color.foreground)
    : Qt.darker(bar ? bar.barForeground : Commons.Color.foreground,1.4)
  Behavior on foreground {
    enabled: !!root.bar && root.bar.foregroundAnimationEnabled
    ColorAnimation { duration: Commons.Style.duration(160) }
  }
}
