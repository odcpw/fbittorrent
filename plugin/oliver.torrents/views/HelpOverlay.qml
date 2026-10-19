import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import qs.Commons as Commons
import qs.Ui as Ui
import "../HelpModel.js" as HelpModel
FocusScope {
 id:root
 required property var service
 readonly property var groups:HelpModel.groups(service.helpActions)
 signal dismissed()
 function listedKeys(){var result=[];for(var i=0;i<sections.count;i++){var section=sections.itemAt(i);for(var j=0;j<section.rows.count;j++){var row=section.rows.itemAt(j);result.push({id:row.modelData.id,keys:row.keyLabel.text,label:row.caption.text,scope:section.modelData.scope})}}return result}
 Keys.priority:Keys.BeforeItem
 Keys.onPressed:function(event){if(event.key===Qt.Key_Escape || event.text==="?"){dismissed();event.accepted=true}else if(event.key===Qt.Key_PageDown){scroll.contentY=Math.min(scroll.contentHeight-scroll.height,scroll.contentY+scroll.height);event.accepted=true}else if(event.key===Qt.Key_PageUp){scroll.contentY=Math.max(0,scroll.contentY-scroll.height);event.accepted=true}}
 Rectangle {anchors.fill:parent;color:Commons.Util.alpha(Commons.Color.background,0.7);MouseArea {anchors.fill:parent;acceptedButtons:Qt.AllButtons}}
 Ui.BorderSurface {
  anchors.centerIn:parent;width:Math.min(parent.width-Commons.Style.space(36),Commons.Style.space(920));height:Math.min(parent.height-Commons.Style.space(36),Commons.Style.space(760))
  color:Commons.Color.popups.background;borderSpec:Commons.Border.surfaceSpec("popups","border",Commons.Color.popups.border,Commons.Style.space(2));radius:Commons.Style.cornerRadius
  ColumnLayout {
   anchors.fill:parent;anchors.margins:Commons.Style.space(18);spacing:Commons.Style.spacing.lg
   RowLayout {Layout.fillWidth:true;Text {Layout.fillWidth:true;text:"Keyboard help";textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.title;font.bold:true} Ui.Button {text:"Close (Esc)";focusable:true;onClicked:root.dismissed()}}
   Flickable {
    id:scroll;Layout.fillWidth:true;Layout.fillHeight:true;clip:true;contentWidth:width;contentHeight:content.implicitHeight;boundsBehavior:Flickable.StopAtBounds
    ScrollBar.vertical:ScrollBar {}
    ColumnLayout {id:content;width:scroll.width;spacing:Commons.Style.spacing.xxl
     Repeater {id:sections;model:root.groups
      ColumnLayout {
       required property var modelData
       property alias rows:items
       Layout.fillWidth:true;spacing:Commons.Style.spacing.sm
       Text {text:modelData.scope;textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.heading;font.bold:true}
       Repeater {id:items;model:modelData.rows
        RowLayout {
         required property var modelData
         property alias keyLabel:keyLabel
         property alias caption:caption
         Layout.fillWidth:true
         Text {id:keyLabel;Layout.preferredWidth:Commons.Style.space(220);text:modelData.keys.dashboard.join(" / ");textFormat:Text.PlainText;color:Commons.Color.foreground;wrapMode:Text.Wrap;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
         Text {id:caption;Layout.fillWidth:true;text:modelData.label+(modelData.note ? " — "+modelData.note : "");textFormat:Text.PlainText;color:Commons.Color.foreground;wrapMode:Text.Wrap;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
        }
       }
      }
     }
    }
   }
  }
 }
}
