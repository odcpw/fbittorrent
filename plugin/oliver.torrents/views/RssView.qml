import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import qs.Commons as Commons
import qs.Ui as Ui
import "../components" as Components
import "../Model.js" as Model
import "../TrackerText.js" as TrackerText

FocusScope {
  id:root
  required property var service
  readonly property var flow:service.rssFlow
  readonly property int revision:service.rssRevision
  readonly property var viewModel:revision>=0 && flow ? flow.snapshot() : ({feeds:[],items:[],pane:"feeds",modal:""})
  readonly property var current:revision>=0 ? flow.current() : null
  readonly property var currentFeed:revision>=0 ? flow.currentFeed() : null
  readonly property bool editing:viewModel.modal==="add"
  readonly property bool dialogOpen:!!viewModel.modal
  readonly property bool narrow:width<Commons.Style.space(1050)
  readonly property var viewState:service.testHooks ? ({rss:viewModel,rssNarrow:narrow,focus:editing ? "rss-input" : viewModel.pane,cancelFocused:viewModel.modal==="remove" && removeConfirm.cancelFocused}) : null
  property string focusedModal:""
  signal changed()
  signal dismissRequested()
  signal focusRequested()
  function focusView(){url.focus=false;name.focus=false;root.forceActiveFocus()}
  function handleKey(event){
    if(viewModel.modal==="add"){
      if(event.key===Qt.Key_Escape){flow.cancel();return true}
      if(event.key===Qt.Key_Return){flow.add(url.text,name.text);return true}
      return false
    }
    if(viewModel.modal==="remove"){
      if(event.key===Qt.Key_Escape){flow.cancel();return true}
      if(event.key===Qt.Key_Left || event.text==="h"){removeConfirm.move(-1);return true}
      if(event.key===Qt.Key_Right || event.text==="l"){removeConfirm.move(1);return true}
      if(event.key===Qt.Key_Return){removeConfirm.activate();return true}
      return true
    }
    if(event.key===Qt.Key_Escape || event.key===Qt.Key_Left || event.text==="h"){flow.back();return true}
    if(event.key===Qt.Key_Return){flow.activate();return true}
    if(event.key===Qt.Key_Right || event.text==="l"){if(viewModel.pane==="feeds")flow.activate();return true}
    if(event.text==="j" || event.key===Qt.Key_Down){flow.move(1);return true}
    if(event.text==="k" || event.key===Qt.Key_Up){flow.move(-1);return true}
    if(event.key===Qt.Key_Home || event.text==="g"){flow.move(0,0);return true}
    if(event.key===Qt.Key_End || event.text==="G"){flow.move(0,-1);return true}
    if(event.key===Qt.Key_PageDown){flow.move(10);return true}
    if(event.key===Qt.Key_PageUp){flow.move(-10);return true}
    if(event.text==="m" || event.text==="M" || event.key===Qt.Key_Space){flow.read(event.text==="M");return true}
    if(event.text==="r" || event.text==="R"){flow.refresh(event.text==="R");return true}
    if(event.text==="n"){flow.addOpen();return true}
    if(event.text==="x" || event.text==="d"){flow.removeOpen();return true}
    if(event.text==="u"){flow.filter();return true}
    if(event.text==="q"){dismissRequested();return true}
    return false
  }
  Keys.priority:Keys.BeforeItem
  Keys.onPressed:function(event){if(root.handleKey(event))event.accepted=true}
  onViewStateChanged:changed()
  Component.onCompleted:flow.mount()
  Component.onDestruction:flow.hide()
  onViewModelChanged:{if(focusedModal===viewModel.modal)return;focusedModal=viewModel.modal;if(viewModel.modal==="add")Qt.callLater(function(){url.text="";name.text="";url.forceActiveFocus()});else if(viewModel.modal==="remove")Qt.callLater(function(){removeConfirm.reset()});else Qt.callLater(function(){root.focusView();root.focusRequested()})}
  ColumnLayout {
    anchors.fill:parent;anchors.margins:Commons.Style.spacing.panelPadding;spacing:Commons.Style.spacing.panelGap
    RowLayout {
      Layout.fillWidth:true
      Text {text:"RSS";textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.title;font.bold:true}
      Item {Layout.fillWidth:true}
      Ui.Button {text:"Feeds";visible:root.narrow && root.viewModel.pane==="items";onClicked:root.flow.back()}
      Ui.Button {text:"Add feed (n)";onClicked:root.flow.addOpen()}
      Ui.Button {text:"Refresh (r)";onClicked:root.flow.refresh(false)}
      Ui.Button {text:root.viewModel.unread ? "Unread only (u)" : "All articles (u)";onClicked:root.flow.filter()}
    }
    RowLayout {
      Layout.fillWidth:true;Layout.fillHeight:true;spacing:Commons.Style.spacing.panelGap
      Components.PaneFrame {
        Layout.preferredWidth:Commons.Style.space(300);Layout.fillWidth:root.narrow;Layout.fillHeight:true
        visible:!root.narrow || root.viewModel.pane==="feeds"
        ColumnLayout {
          anchors.fill:parent
          Text {text:"FEEDS";color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.caption}
          ListView {
            id:feeds;Layout.fillWidth:true;Layout.fillHeight:true;clip:true;keyNavigationEnabled:false;model:root.viewModel.feeds
            currentIndex:root.viewModel.feeds.findIndex(function(row){return row.id===root.viewModel.feed})
            onCurrentIndexChanged:if(currentIndex>=0)positionViewAtIndex(currentIndex,ListView.Contain)
            delegate:Ui.BorderSurface {
              required property var modelData
              width:feeds.width;implicitHeight:Commons.Style.space(68);color:Commons.Color.background
              borderSpec:Commons.Border.controlSpec(modelData.id===root.viewModel.feed ? "focus" : "normal",Commons.Color.foreground,Commons.Color.accent)
              Column {anchors.fill:parent;anchors.margins:Commons.Style.spacing.sm
                Text {width:parent.width;text:modelData.name;textFormat:Text.PlainText;elide:Text.ElideRight;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
                Text {width:parent.width;text:modelData.loading ? "Refreshing…" : modelData.error ? TrackerText.message(modelData.error,modelData.url) : modelData.unread+" unread";textFormat:Text.PlainText;elide:Text.ElideRight;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.caption}
              }
              MouseArea {anchors.fill:parent;onClicked:{root.focusView();root.flow.chooseFeed(modelData.id)} onDoubleClicked:{root.flow.pane="feeds";root.flow.chooseFeed(modelData.id);root.flow.activate()}}
            }
            Components.EmptyState {anchors.centerIn:parent;width:parent.width;visible:!root.viewModel.feeds.length;title:"No feeds yet";message:"Press n to add a feed. Articles never download automatically."}
          }
          Ui.Button {text:"Remove feed (x)";enabled:!!root.currentFeed;onClicked:root.flow.removeOpen()}
        }
      }
      Components.PaneFrame {
        Layout.fillWidth:true;Layout.fillHeight:true;visible:!root.narrow || root.viewModel.pane==="items"
        ColumnLayout {
          anchors.fill:parent;spacing:Commons.Style.spacing.sm
          RowLayout {Layout.fillWidth:true
            Text {Layout.fillWidth:true;text:root.currentFeed ? root.currentFeed.name : "Articles";elide:Text.ElideRight;textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.heading}
            Ui.Button {text:"Mark all read (M)";enabled:!!root.currentFeed;onClicked:root.flow.read(true)}
          }
          ListView {
            id:articles;Layout.fillWidth:true;Layout.fillHeight:true;clip:true;keyNavigationEnabled:false;model:root.viewModel.items
            currentIndex:root.viewModel.items.findIndex(function(row){return row.id===root.viewModel.item})
            onCurrentIndexChanged:if(currentIndex>=0)positionViewAtIndex(currentIndex,ListView.Contain)
            delegate:Ui.BorderSurface {
              required property var modelData
              width:articles.width;implicitHeight:Commons.Style.space(72);color:Commons.Color.background
              borderSpec:Commons.Border.controlSpec(modelData.id===root.viewModel.item && root.viewModel.pane==="items" ? "focus" : "normal",Commons.Color.foreground,Commons.Color.accent)
              Column {anchors.fill:parent;anchors.margins:Commons.Style.spacing.sm
                Text {width:parent.width;text:modelData.title;textFormat:Text.PlainText;elide:Text.ElideRight;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
                Text {width:parent.width;text:(modelData.grabbed ? "Added · " : modelData.read ? "Read · " : "Unread · ")+Model.size(modelData.size,false)+" · "+(modelData.published_at ? new Date(modelData.published_at*1000).toLocaleDateString() : "Date unknown");textFormat:Text.PlainText;elide:Text.ElideRight;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.caption}
              }
              MouseArea {anchors.fill:parent;onClicked:{root.focusView();root.flow.chooseItem(modelData.id)} onDoubleClicked:{root.flow.chooseItem(modelData.id);root.flow.activate()}}
            }
            Components.EmptyState {anchors.centerIn:parent;width:parent.width;visible:!root.viewModel.items.length;title:root.viewModel.loading ? "Loading articles…" : "No articles";message:root.viewModel.unread ? "Press u to include read articles." : "Refresh the feed with r."}
          }
          Text {Layout.fillWidth:true;Layout.maximumHeight:Commons.Style.space(100);text:root.current ? (root.current.description || "") : "";textFormat:Text.PlainText;wrapMode:Text.Wrap;elide:Text.ElideRight;maximumLineCount:4;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
          RowLayout {
            Ui.Button {text:root.current && root.current.read ? "Mark unread (m)" : "Mark read (m)";enabled:!!root.current;onClicked:{root.flow.chooseItem(root.current.id);root.flow.read(false)}}
            Ui.Button {text:"Review download (Enter)";enabled:!!root.current && !!(root.current.magnet || root.current.torrent_url);onClicked:{root.flow.chooseItem(root.current.id);root.flow.activate()}}
          }
        }
      }
    }
    Components.KeyHint {Layout.fillWidth:true;text:TrackerText.message(root.viewModel.message || (root.currentFeed && root.currentFeed.error),root.currentFeed ? root.currentFeed.url : "") || "Enter items / review · m/M read · r/R refresh · n add feed · x remove feed · u unread · ← feeds"}
  }
  Rectangle {anchors.fill:parent;visible:root.dialogOpen;color:Commons.Util.alpha(Commons.Color.background,0.8);MouseArea {anchors.fill:parent;onClicked:{}}}
  Components.PaneFrame {
    anchors.centerIn:parent;width:Math.min(parent.width-Commons.Style.spacing.panelPadding*2,Commons.Style.space(600));height:root.viewModel.modal==="add" ? Commons.Style.space(300) : Commons.Style.space(180);visible:root.dialogOpen
    ColumnLayout {
      anchors.fill:parent;spacing:Commons.Style.spacing.md;visible:root.viewModel.modal==="add"
      Text {text:"Add RSS feed";textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.heading}
      Ui.TextField {id:url;Layout.fillWidth:true;placeholderText:"Feed URL (http:// or https://)";maximumLength:4096}
      Ui.TextField {id:name;Layout.fillWidth:true;placeholderText:"Name (optional)";maximumLength:256}
      Text {Layout.fillWidth:true;visible:!!root.viewModel.message;text:TrackerText.message(root.viewModel.message,url.text);textFormat:Text.PlainText;wrapMode:Text.Wrap;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
      RowLayout {Ui.Button {text:"Cancel";onClicked:root.flow.cancel()} Ui.Button {text:"Add feed";enabled:!!url.text.trim();onClicked:root.flow.add(url.text,name.text)}}
    }
    Components.ConfirmStrip {id:removeConfirm;anchors.fill:parent;visible:root.viewModel.modal==="remove";message:root.currentFeed ? "Remove feed “"+root.currentFeed.name+"”? Downloaded files are kept." : "";confirmText:"Remove feed";onCanceled:root.flow.cancel();onConfirmed:root.flow.remove()}
  }
}
