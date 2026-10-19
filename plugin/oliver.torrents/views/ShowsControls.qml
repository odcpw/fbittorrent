import QtQuick
import qs.Commons as Commons
import qs.Ui as Ui
Flow {
  id:root
  required property var flow
  required property var model
  property string defaultQuality:"1080p"
  readonly property bool popupOpen:read.popupOpen || quality.popupOpen || preference.popupOpen
  readonly property var state:({read:read.value,readOpen:read.popupOpen,quality:quality.value,qualityOpen:quality.popupOpen,preference:preference.value,preferenceOpen:preference.popupOpen})
  signal focusRequested()
  onPopupOpenChanged:if(!popupOpen)Qt.callLater(root.focusRequested)
  spacing:Commons.Style.spacing.md
  function openPreference(){if(preference.enabled)preference.open()}
  Ui.Dropdown {
    id:read;objectName:"shows-read";width:Commons.Style.space(170);label:"Read state";value:root.model.readFilter
    options:[{value:"unread",label:"Unread"},{value:"all",label:"All releases"},{value:"read",label:"Read"}]
    onChanged:function(value){root.flow.filters(value,root.model.qualityFilter)}
  }
  Ui.Dropdown {
    id:quality;objectName:"shows-quality";width:Commons.Style.space(170);label:"Release quality";value:root.model.qualityFilter
    options:[{value:"",label:"All qualities"},"2160p","1080p","720p","480p"]
    onChanged:function(value){root.flow.filters(root.model.readFilter,value)}
  }
  Ui.Dropdown {
    id:preference;objectName:"shows-preference";width:Commons.Style.space(220);label:"Show preference (=)";value:root.model.preference
    enabled:!!root.model.preferenceTarget && !root.model.busy
    options:[{value:"",label:"Default ("+root.defaultQuality+")"},"2160p","1080p","720p","480p"]
    onChanged:function(value){root.flow.setQuality(value)}
  }
  Text {width:root.width;textFormat:Text.PlainText;text:root.model.preferenceTitle ? "Preference for "+root.model.preferenceTitle : "Select a show or release to choose its quality";elide:Text.ElideRight;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.bodySmall}

}
