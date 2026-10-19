import QtQuick
import QtQuick.Layouts
import Qt.labs.folderlistmodel
import qs.Commons as Commons
import qs.Ui as Ui
FocusScope {
  id:root
  required property string initialPath
  property string path:initialPath
  property int cursor:-1
  readonly property bool loading:folders.status===FolderListModel.Loading
  signal selected(string path)
  signal cancelled()
  signal changed()
  // FolderListModel::resolvePath parses the local path as QUrl a second time.
  // Escape both conversions so #, ?, and % remain literal directory characters.
  function localUrl(value){return "file://"+value.split("/").map(function(part){return encodeURIComponent(encodeURIComponent(part))}).join("/")}
  function navigate(value){
    if(typeof value!=="string" || value[0]!=="/" || /[\x00-\x1f\x7f]/.test(value))return false
    path=value.replace(/\/+$/,"") || "/";cursor=-1;address.text=path;changed();return true
  }
  function nextFocus(direction){
    var items=[address,cancelFolder,useFolder],index=items.findIndex(function(item){return item.activeFocus})
    items[(index+direction+items.length)%items.length].forceActiveFocus();changed()
  }
  function parentPath(){return path==="/" ? "/" : path.slice(0,path.lastIndexOf("/")) || "/"}
  function picked(){return cursor>=0 && cursor<folders.count ? String(folders.get(cursor,"filePath")) : path}
  function choose(){if(!loading)selected(picked())}
  function move(amount){cursor=Math.max(-1,Math.min(folders.count-1,cursor+amount));if(cursor>=0)list.positionViewAtIndex(cursor,ListView.Contain);changed()}
  function handleKey(event){
    if(event.key===Qt.Key_Escape){cancelled();return true}
    if(event.key===Qt.Key_Return || event.key===Qt.Key_Enter){if(address.activeFocus)navigate(address.text);else choose();return true}
    if(address.activeFocus)return false
    if(event.key===Qt.Key_Down || event.text==="j"){move(1);return true}
    if(event.key===Qt.Key_Up || event.text==="k"){move(-1);return true}
    if(event.key===Qt.Key_Left || event.key===Qt.Key_Backspace){navigate(parentPath());return true}
    if(event.key===Qt.Key_Right){navigate(picked());return true}
    return false
  }
  function state(){
    var names=[]
    for(var i=0;!loading && i<Math.min(folders.count,16);i++)names.push(folders.get(i,"fileName"))
    return {path:path,url:String(folders.folder),count:folders.count,loading:loading,cursor:cursor,selected:loading ? path : picked(),names:names,field:address.activeFocus ? "address" : cancelFolder.activeFocus ? "cancel" : useFolder.activeFocus ? "use" : "list"}
  }
  Shortcut {sequence:"Tab";context:Qt.WindowShortcut;enabled:root.visible;onActivated:root.nextFocus(1)}
  Shortcut {sequence:"Shift+Tab";context:Qt.WindowShortcut;enabled:root.visible;onActivated:root.nextFocus(-1)}
  Keys.onPressed:function(event){if(root.handleKey(event))event.accepted=true}
  onPathChanged:changed()
  FolderListModel {
    id:folders
    folder:root.localUrl(root.path)
    showDirs:true;showFiles:false;showHidden:false;showDotAndDotDot:false
    sortField:FolderListModel.Name;sortCaseSensitive:false
    onStatusChanged:root.changed()
    onCountChanged:root.changed()
  }
  ColumnLayout {
    anchors.fill:parent;spacing:Commons.Style.spacing.lg
    Text {textFormat:Text.PlainText;text:"Choose download folder";color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.heading;font.bold:true}
    Ui.TextField {id:address;Layout.fillWidth:true;text:root.path;maximumLength:4096;onAccepted:root.navigate(text)}
    Ui.Button {text:"..";tooltipText:root.parentPath();Layout.fillWidth:true;leftAlign:true;enabled:root.path!=="/";onClicked:root.navigate(root.parentPath())}
    ListView {
      id:list;Layout.fillWidth:true;Layout.fillHeight:true;clip:true;reuseItems:true
      model:folders;currentIndex:root.cursor
      boundsBehavior:Flickable.StopAtBounds
      delegate:Ui.Button {
        required property int index
        required property string fileName
        required property string filePath
        width:list.width;text:fileName;tooltipText:filePath;leftAlign:true;selected:index===root.cursor
        onClicked:{root.navigate(filePath);root.forceActiveFocus()}
      }
    }
    Text {Layout.fillWidth:true;textFormat:Text.PlainText;text:root.loading ? "Reading folder…" : "Enter selects · → enters · ← parent";color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.caption}
    RowLayout {
      Layout.fillWidth:true
      Ui.Button {id:cancelFolder;text:"Cancel (Esc)";focusable:true;bordered:true;onClicked:root.cancelled()}
      Item {Layout.fillWidth:true}
      Ui.Button {id:useFolder;text:"Use folder (Enter)";focusable:true;bordered:true;enabled:!root.loading;onClicked:root.choose()}
    }
  }
}
