import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import qs.Ui as Ui
import qs.Commons as Commons
import "../components" as Components
import "../Model.js" as Model
import "../DetailText.js" as DetailText

ScrollView {
  id:root
  required property var service
  required property var torrent
  property var card:null
  readonly property var actionState:actions.state
  readonly property bool posterLoaded:poster.loaded
  signal recheckRequested(string hash)
  clip:true
  contentWidth:availableWidth
  ColumnLayout {
    width:root.availableWidth;spacing:Commons.Style.spacing.sm
    Components.PosterCard {id:poster;Layout.fillWidth:true;card:root.card}
    Components.DetailField {Layout.fillWidth:true;value:root.torrent.name;font.bold:true}
    Components.DetailField {Layout.fillWidth:true;value:Model.stateLabel(root.torrent)+(DetailText.reason(root.torrent,root.service.store.network) ? " · "+DetailText.reason(root.torrent,root.service.store.network) : "")}
    Components.DetailField {Layout.fillWidth:true;visible:!!root.torrent.playable;value:DetailText.playable(root.torrent.playable)}
    Components.DetailField {Layout.fillWidth:true;label:"Size";value:Model.size(root.torrent.size,false)+" · "+Model.percent(root.torrent.progress,root.torrent.state==="done" || root.torrent.state==="seeding")}
    Components.DetailField {Layout.fillWidth:true;label:"Downloaded";value:Model.size(root.torrent.completed_bytes,false)}
    Components.DetailField {Layout.fillWidth:true;label:"Uploaded";value:Model.size(root.torrent.uploaded_bytes,false)}
    Components.DetailField {Layout.fillWidth:true;label:"Ratio · ETA";value:(root.torrent.ratio===undefined ? "—" : Number(root.torrent.ratio).toFixed(2))+" · "+Model.eta(root.torrent.eta_s,false)}
    Components.DetailField {Layout.fillWidth:true;label:"Save";value:root.torrent.save_path || "—"}
    Components.DetailField {Layout.fillWidth:true;visible:!!root.torrent.content_path;label:"Content";value:root.torrent.content_path || ""}
    RowLayout {
      Ui.Button {text:"Copy path";onClicked:root.service.copyText(root.torrent.save_path)}
      Ui.Button {text:"Open content";onClicked:root.service.openTarget(root.torrent.hash)}
    }
    Components.OverviewActions {id:actions;service:root.service;torrent:root.torrent}
    Components.DetailField {Layout.fillWidth:true;label:"Hash";value:root.torrent.hash}
    Ui.Button {text:"Copy hash";onClicked:root.service.copyText(root.torrent.hash)}
    Components.DetailField {Layout.fillWidth:true;label:"Added";value:Model.when(root.torrent.added_at)}
    Components.DetailField {Layout.fillWidth:true;label:"Completed";visible:root.torrent.completed_at!==undefined;value:Model.when(root.torrent.completed_at)}
    Components.DetailField {Layout.fillWidth:true;label:"Undo until";visible:root.torrent.undo_until!==undefined;value:Model.when(root.torrent.undo_until)}
    RowLayout {
      Text {Layout.fillWidth:true;text:"Allow seeding (u)";textFormat:Text.PlainText;color:Commons.Color.foreground;font.family:Commons.Style.font.family;font.pixelSize:Commons.Style.font.body}
      Ui.ToggleSwitch {checked:root.torrent.seed===true;busy:root.service.rowPending(root.torrent.hash);onToggled:root.service.toggleSeed(root.torrent.hash)}
    }
    Components.DetailField {Layout.fillWidth:true;value:DetailText.policy(root.torrent)}
    Components.DetailField {Layout.fillWidth:true;label:"Queue position";visible:root.torrent.queue_pos!==undefined;value:String(root.torrent.queue_pos)}
    Components.DetailField {Layout.fillWidth:true;label:"Files · Pieces";value:Model.count(root.torrent.file_count)+" · "+Model.count(root.torrent.piece_count)+" × "+Model.size(root.torrent.piece_size,false)}
    Components.DetailField {Layout.fillWidth:true;visible:root.torrent.private!==undefined;value:root.torrent.private ? "Private torrent" : "Public torrent"}
    Components.DetailField {Layout.fillWidth:true;visible:!!root.torrent.comment;label:"Comment";value:root.torrent.comment || ""}
    Components.DetailField {Layout.fillWidth:true;visible:!!root.torrent.created_by;label:"Created by";value:root.torrent.created_by || ""}
    RowLayout {
      Ui.Button {text:"IMDb (i)";onClicked:root.service.openUrl(root.card ? root.card.url : "https://www.imdb.com/find/?q="+encodeURIComponent(root.torrent.name))}
      Ui.Button {text:"Wikipedia (w)";onClicked:root.service.openUrl(root.card && root.card.wiki_url ? root.card.wiki_url : "https://en.wikipedia.org/w/index.php?search="+encodeURIComponent(root.torrent.name))}
    }
    RowLayout {
      Ui.Button {visible:root.service.hasCap("recheck");text:"Recheck…";onClicked:root.recheckRequested(root.torrent.hash)}
    }
    Item {Layout.preferredHeight:Commons.Style.spacing.sm}
  }
}
