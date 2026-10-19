import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import qs.Commons as Commons
import qs.Ui as Ui
import "../components" as Components

FocusScope {
  id:root
  required property var service
  property string initialQuery:""
  property string lastBrowseLabel:""
  property alias queryTarget:queryInput
  readonly property var flow:service.searchFlow
  readonly property int revision:service.searchRevision
  readonly property var viewModel:revision>=0 && flow ? flow.snapshot() : ({rows:[],phase:"idle",sort:{key:"seeds",reverse:false}})
  readonly property bool editing:queryInput.activeFocus || (sheet.item && sheet.item.editing===true)
  readonly property bool dialogOpen:!!viewModel.panel
  readonly property var viewState:({query:queryInput.text,cursor:viewModel.cursor,sort:viewModel.sort,rows:viewModel.rows,search:{id:viewModel.session,frames:viewModel.frames,total:viewModel.total,phase:viewModel.phase,mode:viewModel.mode,label:viewModel.label},panel:viewModel.panel,panelCount:viewModel.panel==="providers" ? flow.providers.length : viewModel.panel==="browse" ? flow.sources.length : 0,added:viewModel.rows.filter(function(r){return !!r.added}).map(function(r){return r.key}),poster:viewModel.poster,posterLoaded:posterCard.loaded,rating:flow.current() ? flow.rating(flow.current()) : undefined,focus:editing ? "query" : "results"})
  signal changed()
  signal queryEdited(string query)
  signal dismissRequested()
  function focusQuery(){queryInput.forceActiveFocus()}
  function focusResults(){service.cancelFocusTarget(queryInput);queryInput.focus=false;root.forceActiveFocus()}
  function open(query,run){queryInput.text=String(query || "");if(run && queryInput.text)flow.start(queryInput.text);Qt.callLater(focusQuery)}
  function key(event){
    if(sheet.item){if(sheet.item.key(event))return true;return sheet.item.editing===true ? false : true}
    if(editing){
      if(event.key===Qt.Key_Return){flow.start(queryInput.text);focusResults();return true}
      if(event.key===Qt.Key_Escape && queryInput.text){flow.cancel();queryInput.text="";queryEdited("");return true}
      if(event.key===Qt.Key_Escape || event.key===Qt.Key_Down){focusResults();return true}
      return false
    }
    if(event.key===Qt.Key_Escape){if(viewModel.phase==="searching"){flow.cancel();return true}focusQuery();return true}
    if(event.text==="/" || event.text==="e"){focusQuery();return true}
    if(event.text==="j" || event.key===Qt.Key_Down){flow.move(1);return true}
    if(event.text==="k" || event.key===Qt.Key_Up){flow.move(-1);return true}
    if(event.key===Qt.Key_Home || event.text==="g"){flow.move(0,0);return true}
    if(event.key===Qt.Key_End || event.text==="G"){flow.move(0,viewModel.rows.length-1);return true}
    if(event.key===Qt.Key_PageDown){flow.move(Math.max(1,Math.floor(list.height/Commons.Style.space(50))));return true}
    if(event.key===Qt.Key_PageUp){flow.move(-Math.max(1,Math.floor(list.height/Commons.Style.space(50))));return true}
    if(event.text==="s"){flow.cycleSort();return true}
    if(event.text==="S"){flow.reverseSort();return true}
    if(event.text==="P"){flow.providersOpen();return true}
    if(event.text==="b"){flow.browseOpen();return true}
    if(event.key===Qt.Key_Return){flow.activate();return true}
    if(event.text==="y"){flow.copy();return true}
    if(event.text==="O"){flow.link("source");return true}
    if(event.text==="i" || event.text==="w"){flow.link(event.text==="i" ? "imdb" : "wiki");return true}
    if(event.text==="f"){flow.follow();return true}
    if(event.text==="q"){dismissRequested();return true}
    return false
  }
  function handleKey(event){return key(event)}
  Keys.priority:Keys.BeforeItem
  Keys.onPressed:function(event){if(root.key(event))event.accepted=true}
  onViewModelChanged:{
    if(viewModel.mode==="browse" && viewModel.label!==lastBrowseLabel){lastBrowseLabel=viewModel.label;queryInput.text=viewModel.label;focusResults()}
    else if(viewModel.mode==="query")lastBrowseLabel=""
  }
  onViewStateChanged:changed()
  Component.onCompleted:{flow.mount();open(initialQuery,!!initialQuery)}
  Component.onDestruction:flow.hide()
  ColumnLayout {
    anchors.fill:parent;anchors.margins:Commons.Style.spacing.panelPadding;spacing:Commons.Style.spacing.panelGap
    RowLayout {
      Layout.fillWidth:true;spacing:Commons.Style.spacing.sm
      Ui.BorderSurface {
        Layout.fillWidth:true;implicitHeight:Commons.Style.space(36);color:Commons.Color.background
        borderSpec:Commons.Border.controlSpec(root.editing ? "focus" : "normal",Commons.Color.foreground,Commons.Color.accent)
        Ui.FilterInput {
          id:queryInput
          anchors.fill:parent;anchors.margins:Commons.Style.spacing.sm;maximumLength:256
          font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body;color:Commons.Color.foreground
          onTextEdited:root.queryEdited(text)
          onActiveFocusChanged:root.changed()
          Keys.onPressed:function(event){if(root.key(event))event.accepted=true}
          Text {visible:!parent.text;text:"Search titles · Enter";textFormat:Text.PlainText;color:Commons.Color.foreground;font:parent.font}
        }
      }
      Ui.Button {text:"Search";enabled:!!queryInput.text.trim();onClicked:{root.flow.start(queryInput.text);root.focusResults()}}
      Ui.Button {text:"Browse (b)";onClicked:root.flow.browseOpen()}
      Ui.Button {text:"Providers (P)";onClicked:root.flow.providersOpen()}
    }
    RowLayout {
      Layout.fillWidth:true;Layout.fillHeight:true;spacing:Commons.Style.spacing.panelGap
      Components.PaneFrame {
        Layout.fillWidth:true;Layout.fillHeight:true
        ColumnLayout {
          anchors.fill:parent;spacing:Commons.Style.spacing.xxs
          RowLayout {
            Layout.fillWidth:true;spacing:Commons.Style.spacing.sm
            Text {Layout.fillWidth:true;text:"NAME";textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.caption}
            Repeater {model:[{label:"SIZE",extent:76},{label:"SEEDS",extent:62},{label:"★",extent:52},{label:"UPLOADED",extent:110},{label:"PROVIDER",extent:130}];Text {required property var modelData;Layout.preferredWidth:Commons.Style.space(modelData.extent);text:modelData.label;textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.caption}}
          }
          ListView {
            id:list
            Layout.fillWidth:true;Layout.fillHeight:true;clip:true;keyNavigationEnabled:false
            model:root.viewModel.rows
            currentIndex:root.viewModel.rows.findIndex(function(r){return r.key===root.viewModel.cursor})
            onCurrentIndexChanged:if(currentIndex>=0)positionViewAtIndex(currentIndex,ListView.Contain)
            delegate:Components.SearchRow {required property var modelData;width:list.width;row:modelData;ratingValue:root.revision>=0 ? root.flow.rating(modelData) : undefined;selected:modelData.key===root.viewModel.cursor;onChosen:{root.focusResults();root.flow.choose(modelData.key)} onActivated:root.flow.activate()}
            Components.EmptyState {anchors.centerIn:parent;width:Math.min(parent.width,Commons.Style.space(420));visible:!root.viewModel.rows.length;title:root.viewModel.phase==="searching" ? "Searching…" : root.viewModel.phase==="idle" ? "Search for a title" : root.viewModel.phase==="stopped" ? "Search stopped" : root.viewModel.phase==="error" ? "Search failed" : "No results for “"+(root.viewModel.label || "")+"”";message:root.viewModel.phase==="error" ? root.viewModel.message : "Enter a title above, or press b to browse lists."}
          }
        }
      }
      Components.PaneFrame {
        visible:root.width>=Commons.Style.space(1100) && !!root.viewModel.poster
        Layout.preferredWidth:Commons.Style.space(280);Layout.fillHeight:true
        Components.PosterCard {id:posterCard;anchors.top:parent.top;width:parent.width;card:root.revision>=0 ? root.flow.card : null}
      }
    }
    Components.KeyHint {Layout.fillWidth:true;text:(root.viewModel.total>500 ? "Showing 500 of "+root.viewModel.total+" · " : "")+(root.viewModel.phase==="searching" ? "Providers still searching · " : "")+"Sort: "+root.viewModel.sort.key+(root.viewModel.sort.reverse ? " reversed" : "")+" · "+(root.viewModel.message || "")}
    Components.KeyHint {Layout.fillWidth:true;text:"Enter review add · s sort · S reverse · f follow · i IMDb · O source · y magnet · b browse · P providers · / query"}
  }
  Rectangle {anchors.fill:parent;visible:!!root.viewModel.panel;color:Commons.Util.alpha(Commons.Color.background,0.7);MouseArea {anchors.fill:parent;onClicked:root.flow.closePanel()}}
  Components.PaneFrame {
    visible:!!root.viewModel.panel
    anchors.centerIn:parent;width:Math.min(parent.width-2*Commons.Style.spacing.panelGap,Commons.Style.space(680));height:Math.min(parent.height-2*Commons.Style.spacing.panelGap,Commons.Style.space(600))
    MouseArea {anchors.fill:parent;onClicked:{}}
    Loader {id:sheet;anchors.fill:parent;active:!!root.viewModel.panel;sourceComponent:root.viewModel.panel==="providers" ? providerComponent : browseComponent}
  }
  Component {id:providerComponent;SearchProviders {flow:root.flow;revision:root.revision;onCloseRequested:root.flow.closePanel()}}
  Component {id:browseComponent;SearchBrowse {flow:root.flow;revision:root.revision;onCloseRequested:root.flow.closePanel()}}
}
