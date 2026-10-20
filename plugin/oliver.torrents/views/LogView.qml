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
 property int lastVersion:-1
 property string lastFilter:""
 property var sourceRows:[]
 readonly property var sources:[""].concat(sourceRows)
 ListModel {id:logModel}
 function refresh(){
  if(!service || !service.store)return
  var store=service.store,signature=JSON.stringify([level,source,hashFilter])
  if(lastVersion===store.logVersion && lastFilter===signature)return
  if(lastFilter!==signature || lastVersion!==store.logVersion-1){
   logModel.clear();LogModel.select(store.logEntries(""),level,source,hashFilter).forEach(function(row){logModel.append({entry:row})})
  }else{
   var removed=store.logDelta.removed
   while(logModel.count && removed.indexOf(logModel.get(0).entry.seq)>=0)logModel.remove(0)
   LogModel.select(store.logDelta.added,level,source,hashFilter).forEach(function(row){logModel.append({entry:row})})
  }
  sourceRows=store.logSources();lastVersion=store.logVersion;lastFilter=signature;Qt.callLater(scrollEnd);changed()
 }
 Connections {target:root.service;function onLogRevisionChanged(){root.refresh()}}
 readonly property int count:list.count
 signal changed()
 function rendered(){var result=[];for(var i=0;i<list.contentItem.children.length;i++){var child=list.contentItem.children[i];if(child.entry && child.text)result.push({seq:child.entry.seq,text:child.text})}return result}
 function scrollEnd(){if(follow)list.positionViewAtEnd()}
 onHashChanged:hashFilter=hash
 onLevelChanged:refresh()
 onSourceChanged:refresh()
 onHashFilterChanged:refresh()
 onConsumingChanged:service.logConsumer(consumer,consuming)
 Component.onCompleted:{service.logConsumer(consumer,consuming);refresh()}
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
   id:list;Layout.fillWidth:true;Layout.fillHeight:true;clip:true;spacing:Commons.Style.spacing.sm;model:logModel
   onCountChanged:Qt.callLater(root.scrollEnd)
   onMovementStarted:root.follow=false
   ScrollBar.vertical:ScrollBar {}
   delegate:Text {
    required property var entry
    width:ListView.view.width
    text:Model.when(entry.ts)+" · "+entry.level+" · "+entry.source+"\n"+entry.message
    textFormat:Text.PlainText;wrapMode:Text.Wrap;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body
   }
   MouseArea {anchors.fill:parent;acceptedButtons:Qt.NoButton;onWheel:function(wheel){if(wheel.angleDelta.y>0)root.follow=false;wheel.accepted=false}}
   Text {anchors.centerIn:parent;width:parent.width;wrapMode:Text.Wrap;text:"No log entries match these filters";visible:root.count===0;textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
  }
 }
}
