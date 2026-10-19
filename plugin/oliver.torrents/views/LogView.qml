import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import qs.Commons as Commons
import qs.Ui as Ui
import "../Model.js" as Model
import "../LogModel.js" as LogModel
FocusScope {
 id:root
 required property var service
 property string hash:""
 property string consumer:"log-view"
 property bool consuming:visible
 property string level:""
 property string source:""
 property string hashFilter:hash
 property bool follow:true
 readonly property var allEntries:service.logRevision>=0 ? service.store.logEntries("") : []
 readonly property var entries:LogModel.select(allEntries,level,source,hashFilter)
 readonly property var sources:[""].concat(LogModel.sources(allEntries))
 readonly property int count:list.count
 signal changed()
 function rendered(){var result=[];for(var i=0;i<list.contentItem.children.length;i++){var child=list.contentItem.children[i];if(child.modelData && child.text)result.push({seq:child.modelData.seq,text:child.text})}return result}
 function scrollEnd(){if(follow)list.positionViewAtEnd()}
 onHashChanged:hashFilter=hash
 onEntriesChanged:{Qt.callLater(scrollEnd);changed()}
 onLevelChanged:changed()
 onSourceChanged:changed()
 onHashFilterChanged:changed()
 onConsumingChanged:service.logConsumer(consumer,consuming)
 Component.onCompleted:service.logConsumer(consumer,consuming)
 Component.onDestruction:service.logConsumer(consumer,false)
 ColumnLayout {
  anchors.fill:parent;anchors.margins:root.consumer==="global-log" ? Commons.Style.spacing.panelPadding : 0;spacing:Commons.Style.spacing.sm
  Text {visible:root.consumer==="global-log";text:"Log";textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.heading;font.bold:true}
  Flow {
   Layout.fillWidth:true;spacing:Commons.Style.spacing.xs
   Repeater {model:["","trace","debug","info","warn","error"]
    Ui.Button {required property string modelData;objectName:"log-level-"+(modelData || "all");text:(root.level===modelData ? "• " : "")+(modelData || "All levels");focusable:true;onClicked:root.level=modelData}
   }
  }
  Flow {
   Layout.fillWidth:true;spacing:Commons.Style.spacing.xs
   Repeater {model:root.sources
    Ui.Button {required property string modelData;objectName:"log-source-"+(modelData || "all");text:(root.source===modelData ? "• " : "")+(modelData || "All sources");focusable:true;onClicked:root.source=modelData}
   }
  }
  RowLayout {
   Layout.fillWidth:true
   Ui.TextField {Layout.fillWidth:true;placeholderText:"Source (exact)";text:root.source;maximumLength:128;onTextEdited:root.source=text}
   Ui.TextField {Layout.fillWidth:true;placeholderText:"Torrent hash (optional)";text:root.hashFilter;maximumLength:40;onTextEdited:root.hashFilter=text}
   Ui.Button {text:root.follow ? "Following" : "Follow newest";focusable:true;onClicked:{root.follow=true;root.scrollEnd()}}
  }
  ListView {
   id:list;Layout.fillWidth:true;Layout.fillHeight:true;clip:true;spacing:Commons.Style.spacing.sm;model:root.entries
   onCountChanged:Qt.callLater(root.scrollEnd)
   onMovementStarted:root.follow=false
   ScrollBar.vertical:ScrollBar {}
   delegate:Text {
    required property var modelData
    width:ListView.view.width
    text:Model.when(modelData.ts)+" · "+modelData.level+" · "+modelData.source+"\n"+modelData.message
    textFormat:Text.PlainText;wrapMode:Text.Wrap;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body
   }
   MouseArea {anchors.fill:parent;acceptedButtons:Qt.NoButton;onWheel:function(wheel){if(wheel.angleDelta.y>0)root.follow=false;wheel.accepted=false}}
   Text {anchors.centerIn:parent;width:parent.width;wrapMode:Text.Wrap;text:"No log entries match these filters";visible:root.count===0;textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
  }
 }
}
