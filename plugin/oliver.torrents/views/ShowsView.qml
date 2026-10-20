import QtQuick
import QtQuick.Layouts
import qs.Commons as Commons
import qs.Ui as Ui
import "../components" as Components
FocusScope {
  id:root
  required property var service
  readonly property var flow:service.showsFlow
  readonly property int revision:service.showsRevision
  readonly property var model:revision>=0 ? flow.snapshot() : ({shows:[],rows:[]})
  readonly property bool editing:followInput.activeFocus || controls.popupOpen
  readonly property bool dialogOpen:model.panel!=="" || controls.popupOpen
  readonly property bool narrow:width<Commons.Style.space(1050)
  function snapshot(){return {shows:model,controls:controls.state,focus:followInput.activeFocus ? "follow" : controls.popupOpen ? "controls" : model.focus,unfollowCancel:confirmation.cancelFocused}}
  signal changed()
  signal focusRequested()
  Item {id:resultsFocus;focus:true}
  Connections {target:controls;function onPopupOpenChanged(){if(controls.popupOpen)root.service.cancelFocusTarget(root)}}
  function focusResults(){followInput.focus=false;resultsFocus.forceActiveFocus();root.focusRequested()}
  function handleKey(event){
    if(controls.popupOpen)return false
    if(event.text==="F" && !root.dialogOpen){controls.openFilters();return true}
    if(event.text==="=" && !root.dialogOpen){controls.openPreference();return true}
    if(model.panel==="follow"){
      if(event.key===Qt.Key_Escape){flow.cancel();root.forceActiveFocus();return true}
      if(event.key===Qt.Key_Return){flow.follow(followInput.text);return true}
      return false
    }
    if(model.panel==="remove"){
      if(event.key===Qt.Key_Escape){flow.cancel();root.forceActiveFocus();return true}
      if(event.key===Qt.Key_Left || event.text==="h"){confirmation.move(-1);return true}
      if(event.key===Qt.Key_Right || event.text==="l"){confirmation.move(1);return true}
      if(event.key===Qt.Key_Return){confirmation.activate();return true}
      return true
    }
    if(event.text==="j" || event.key===Qt.Key_Down){flow.move(1);return true}
    if(event.text==="k" || event.key===Qt.Key_Up){flow.move(-1);return true}
    if(event.text==="g" || event.key===Qt.Key_Home){flow.move(0,0);return true}
    if(event.text==="G" || event.key===Qt.Key_End){flow.move(0,model.focus==="shows" ? model.shows.length : model.rows.length-1);return true}
    if(event.text==="h" || event.key===Qt.Key_Left || event.key===Qt.Key_Escape){flow.pane(false);return true}
    if(event.text==="l" || event.key===Qt.Key_Right){flow.pane(true);return true}
    if(event.key===Qt.Key_Return){flow.primary();return true}
    if(event.text==="n" || event.text==="+"){flow.followOpen();return true}
    if(event.text==="m" || event.key===Qt.Key_Space){flow.mark();return true}
    if(event.text==="M"){flow.markAll();return true}
    if(event.text==="u"){flow.undo();return true}
    if(event.text==="r" || event.text==="R"){flow.check(event.text==="R");return true}
    if(event.text==="x" || event.text==="d"){flow.removeOpen();return true}
    return false
  }
  Keys.priority:Keys.BeforeItem
  Keys.onPressed:function(event){if(handleKey(event))event.accepted=true}
  property string previousPanel:""
  onModelChanged:{if(model.panel!==previousPanel){previousPanel=model.panel;Qt.callLater(function(){if(model.panel==="follow")followInput.forceActiveFocus();else if(model.panel==="remove")confirmation.reset();else {root.service.cancelFocusTarget(followInput);root.focusResults()}})}}
  Component.onCompleted:flow.mount()
  Component.onDestruction:flow.hide()
  ColumnLayout {
    anchors.fill:parent;anchors.margins:Commons.Style.spacing.panelPadding;spacing:Commons.Style.spacing.md
    RowLayout {
      Layout.fillWidth:true
      Ui.Button {text:root.narrow && root.model.focus==="releases" ? "‹ Shows / "+(root.model.selected==="inbox" ? "Unread inbox" : "Releases") : "Shows";onClicked:root.flow.pane(false)}
      Ui.Button {text:"Follow (n)";enabled:!root.model.busy;onClicked:root.flow.followOpen()}
      Ui.Button {text:"Check (r)";enabled:!root.model.busy;onClicked:root.flow.check(false)}
      Ui.Button {text:"Undo read (u)";enabled:!root.model.busy;onClicked:root.flow.undo()}
      Item {Layout.fillWidth:true}
      Text {textFormat:Text.PlainText;text:root.model.busy ? "Working…" : "Enter review · m read/unread · M all read";color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.bodySmall}
    }
    ShowsControls {id:controls;Layout.fillWidth:true;flow:root.flow;model:root.model;defaultQuality:root.service.store.settings.show_quality || "1080p";onFocusRequested:root.focusResults()}
    Text {visible:root.model.error!=="";text:root.model.error;textFormat:Text.PlainText;Layout.fillWidth:true;wrapMode:Text.Wrap;color:Commons.Color.urgent;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
    RowLayout {
      Layout.fillWidth:true;Layout.fillHeight:true;spacing:Commons.Style.spacing.panelGap
      ListView {
        id:shows;visible:!root.narrow || root.model.focus==="shows";Layout.preferredWidth:root.narrow ? root.width : Commons.Style.space(300);Layout.fillHeight:true;clip:true
        model:[{imdb:"inbox",title:"Unread inbox",unread:root.model.shows.reduce(function(n,s){return n+s.unread},0)}].concat(root.model.shows)
        currentIndex:root.model.selected==="inbox" ? 0 : Math.max(0,root.model.shows.findIndex(function(s){return s.imdb===root.model.selected})+1)
        onCurrentIndexChanged:positionViewAtIndex(currentIndex,ListView.Contain)
        delegate:Rectangle {
          required property var modelData
          width:shows.width;height:Commons.Style.spacing.controlHeight
          color:modelData.imdb===root.model.selected ? Commons.Style.selectionFill : Commons.Util.alpha(Commons.Color.background,0)
          Text {anchors.fill:parent;anchors.margins:Commons.Style.spacing.sm;textFormat:Text.PlainText;text:modelData.title+"  "+modelData.unread+(modelData.checking ? " · checking…" : "");elide:Text.ElideRight;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
          MouseArea {anchors.fill:parent;onClicked:{root.flow.select(parent.modelData.imdb,false);root.forceActiveFocus()}}
        }
      }
      ColumnLayout {
        visible:!root.narrow || root.model.focus==="releases";Layout.fillWidth:true;Layout.fillHeight:true
        Text {textFormat:Text.PlainText;text:root.model.selected==="inbox" ? (root.model.readFilter==="all" ? "All releases" : root.model.readFilter==="read" ? "Read releases" : "Unread releases") : "Releases · read state / transfer state";color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.title}
        ListView {
          id:releases;Layout.fillWidth:true;Layout.fillHeight:true;clip:true;model:root.model.rows;currentIndex:root.model.cursor
          onCurrentIndexChanged:positionViewAtIndex(currentIndex,ListView.Contain)
          delegate:Rectangle {
            required property var modelData
            required property int index
            width:releases.width;height:Commons.Style.space(62)
            color:index===root.model.cursor && root.model.focus==="releases" ? Commons.Style.selectionFill : Commons.Util.alpha(Commons.Color.background,0)
            Column {anchors.fill:parent;anchors.margins:Commons.Style.spacing.sm
              Text {width:parent.width;textFormat:Text.PlainText;text:(modelData.read ? "○ Read  " : "● Unread  ")+modelData.title;elide:Text.ElideRight;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
              Text {textFormat:Text.PlainText;text:modelData.key+" · "+(modelData.quality || "Quality unknown")+" · Transfer: "+(modelData.transfer==="none" ? "not added" : modelData.transfer);color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.bodySmall}
            }
            MouseArea {anchors.fill:parent;onClicked:{root.flow.pane(true);root.flow.move(0,parent.index);root.forceActiveFocus()} onDoubleClicked:root.flow.primary()}
          }
        }
        Text {visible:root.model.rows.length===0;textFormat:Text.PlainText;text:root.model.loading ? "Loading releases…" : root.model.readFilter==="unread" && !root.model.qualityFilter ? "No unread releases. Check for updates, or change the filters to see read releases." : "No releases match these filters.";wrapMode:Text.Wrap;Layout.fillWidth:true;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
      }
    }
  }
  Rectangle {
    anchors.fill:parent;visible:root.model.panel!=="";color:Commons.Util.alpha(Commons.Color.background,0.85)
    MouseArea {anchors.fill:parent}
    Ui.BorderSurface {
      anchors.centerIn:parent;width:Math.min(parent.width-Commons.Style.space(40),Commons.Style.space(500));height:dialog.implicitHeight+Commons.Style.space(40);color:Commons.Color.popups.background
      borderSpec:Commons.Border.surfaceSpec("popups","border",Commons.Color.popups.border,Commons.Style.space(2));padding:Commons.Style.space(20)
      ColumnLayout {id:dialog;anchors.fill:parent;anchors.margins:Commons.Style.space(20)
        Text {textFormat:Text.PlainText;text:root.model.panel==="follow" ? "Follow a show" : "Unfollow show? Downloads are kept.";color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.title;wrapMode:Text.Wrap;Layout.fillWidth:true}
        Ui.TextField {id:followInput;visible:root.model.panel==="follow";Layout.fillWidth:true;maximumLength:256;placeholderText:"Show title or IMDb ID";onAccepted:root.flow.follow(text)}
        Ui.Button {visible:root.model.panel==="follow";text:"Follow";enabled:!root.model.busy;onClicked:root.flow.follow(followInput.text)}
        Components.ConfirmStrip {id:confirmation;visible:root.model.panel==="remove";Layout.fillWidth:true;message:"Unfollowing removes this show from the inbox. Existing downloads remain.";confirmText:"Unfollow";onCanceled:root.flow.cancel();onConfirmed:root.flow.remove()}
      }
    }
  }
}
