import QtQuick
import Quickshell
import Quickshell.Io
import Quickshell.Wayland
import Quickshell.Hyprland
import qs.Commons as Commons
import "ApiClient.js" as ApiClient
import "Store.js" as Store
import "Model.js" as Model
import "PopupState.js" as PopupState
import "Focus.js" as Focus
import "AddFlow.js" as AddFlow
import "ClipboardFlow.js" as ClipboardFlow
import "SettingsFlow.js" as SettingsFlow
import "AddSource.js" as AddSource
import "DetailFlow.js" as DetailFlow
import "OverviewFlow.js" as OverviewFlow
import "CleanupFlow.js" as CleanupFlow
import "SearchFlow.js" as SearchFlow
import "BrowsePosters.js" as BrowsePosters

Item {
  id: root
  property var shell: null
  property var manifest: ({version: "0.2.0"})
  property var settings: ({})
  property bool popupOpen: false
  property bool dashboardVisible: false
  property var dashboardState:null
  property var dashboardOwner:null
  property var windowFocus:null
  property int windowFocusSteps:0
  property var detailTopics: []
  property var logConsumers:({})
  property var logTopics:[]
  property var detailFlow:null
  property int detailRevision:0
  property int logRevision:0
  property real detailsWidth:360
  property var savedPreferences:({})
  onDetailsWidthChanged:if(preferencesLoaded)savePreferences.restart()
  function hasCap(name){return !!client && client.caps.indexOf(name)>=0}
  function subscribeDetail(hash,topics){detailTopics=topics.map(function(t){return "torrent:"+hash+"."+t})}
  function unsubscribeDetail(){if(detailTopics.length){var old=detailTopics;detailTopics=[];store.releaseTopics(old.filter(function(t){return addTopics.indexOf(t)<0}))}}
  function logConsumer(id,on){
    if(!!logConsumers[id]===!!on)return
    var next=Object.assign({},logConsumers);if(on)next[id]=true;else delete next[id];logConsumers=next
    var wanted=Object.keys(next).length>0 && hasCap("log")
    if(wanted!==!!logTopics.length){logTopics=wanted ? ["log"] : [];if(wanted)client.request("app.log",{limit:1000})}
  }
  function setFiles(hash,indexes,included,callback){var args={hash:hash};args[included ? "include" : "skip"]=indexes;return mutate("torrents.set_files",args,"files",callback)}
  function lookupImdb(args,callback){return hasCap("imdb") ? client.request("imdb.lookup",args,{callback:callback,quiet:true}) : false}
  function copyText(text){if(text!==undefined){Commons.Util.execArgv(["wl-copy","--",String(text)]);notify("Copied")}}
  function openUrl(url){if(url && /^https?:\/\//.test(url))Commons.Util.execArgv(["xdg-open",url])}
  function scheduleDetail(hash){detailChoice.hash=hash;detailChoice.restart();imdbChoice.restart()}
  function hideDetail(){detailChoice.stop();imdbChoice.stop();if(detailFlow)detailFlow.hide()}
  Timer {id:detailChoice;property string hash:"";interval:150;onTriggered:if(root.detailFlow)root.detailFlow.begin(hash)}
  Timer {id:imdbChoice;interval:300;onTriggered:if(root.detailFlow)root.detailFlow.lookup()}

  property var overviewFlow:null
  property int cleanupRevision:0
  property var cleanupFlow:null
  property int overviewRevision:0
  property var searchTopics:[]
  property var searchFlow:null
  property var browsePosters:null
  property int browsePosterRevision:0
  property int searchRevision:0
  function scheduleSearchPoster(){searchPoster.restart()}
  Timer {id:searchPoster;interval:300;onTriggered:if(root.searchFlow)root.searchFlow.lookup()}
  function beginSearchAdd(row,session){return addFlow ? addFlow.search(row,session) : false}
  property var addTopics:[]
  property var addFlow:null
  property int addRevision:0
  property bool clipboardForSheet:false
  function beginAdd(source,options){if(addFlow)return addFlow.open(source,options);return false}
  function addClipboardSheet(){if(!beginAdd(null))return;clipboardForSheet=true;addFromClipboard()}
  function finishAdd(hash){if(dashboardOwner && hash)dashboardOwner.selectTorrent(hash)}
  property string runtimeOverride: ""
  readonly property string runtimeDir: runtimeOverride || Quickshell.env("XDG_RUNTIME_DIR")
  readonly property string bin: String(settings.binary || "fbittorrent")
  readonly property bool online: status.ok === true && status.running !== false
  property bool resultUnknown:false
  property bool reconnecting: false
  property bool stoppedRecently: false
  property string byeReason: ""
  property bool startupRequested: false
  property var clipboardOffer: null
  property var clipboardFlow:null
  property var settingsFlow:null
  property int settingsRevision:0
  property var helpActions:[]
  property string instance: ""
  property string backendState: "stopped"
  property string lastError: ""
  property real lastMessageAt:0
  property string lastMessageText:""
  property string actionStatus: ""
  property var pending: []
  readonly property bool busy: pending.length > 0
  property var torrents: []
  property var badgeCounts:({})
  property string sortKey: "added"
  property bool sortReverse: false
  property bool preferencesLoaded:false
  property int actionRevision:0
  property int pendingFrame:0
  property var toast:null
  property real now:Date.now()
  function sortChanged() {syncRows();if(preferencesLoaded)savePreferences.restart()}
  function cycleSort() {
    var keys=["added","name","progress","rate","size","state","eta"]
    sortKey=keys[(keys.indexOf(sortKey)+1)%keys.length];sortReverse=false
  }
  function sortBy(key) {if(sortKey===key)sortReverse=!sortReverse;else{sortKey=key;sortReverse=false}}
  function rowPending(hash) {var revision=actionRevision;return !!store.pendingByHash[hash]}
  function rowFailed(hash) {var revision=actionRevision;return !!store.failedByHash[hash]}
  function pendingAction(hash) {
    if(!client)return null
    var actions=client.actions,refs=Object.keys(actions)
    for(var i=0;i<refs.length;i++){var action=actions[refs[i]],args=action.args || {};if(action.status==="pending" && ((args.hashes || []).indexOf(hash)>=0 || args.hash===hash))return action}
    return null
  }
  function pendingGlyph(hash) {var revision=actionRevision;return store.pendingByHash[hash] ? (Commons.Style.reduceMotion ? "…" : "⠋⠙⠹⠸⠼⠴⠦⠧"[pendingFrame]) : ""}
  function clearFailures(){if(client){store.clearFailures(client.actions);actionRevision++}}
  function notify(text){lastMessageText=text;lastMessageAt=Date.now();toast={text:text,until:Date.now()+5000};toastTimer.restart()}
  function copyMagnet(hash){copyText("magnet:?xt=urn:btih:"+hash)}
  function titleLink(hash,wikipedia) {
    var row=store.torrents[hash]
    if(!row)return
    var card=detailFlow && detailFlow.hash===hash ? detailFlow.card : null
    var url=card && (wikipedia ? card.wiki_url : card.url) ? (wikipedia ? card.wiki_url : card.url) : wikipedia ? "https://en.wikipedia.org/w/index.php?search="+encodeURIComponent(row.name) : row.imdb_id ? "https://www.imdb.com/title/"+row.imdb_id+"/" : "https://www.imdb.com/find/?q="+encodeURIComponent(row.name)
    Quickshell.execDetached(["xdg-open",url])
  }
  function follow(hash) {
    var row=store.torrents[hash]
    if(!row)return false
    if(client.caps.indexOf("shows")<0){notify("Shows is unavailable; update the daemon");return false}
    return mutate("shows.follow",row.imdb_id ? {imdb:row.imdb_id} : {query:row.name},"shows")
  }
  readonly property alias torrentModel: rowsModel
  ListModel { id: rowsModel; dynamicRoles: true }
  property bool modelReordering:false
  function orderBoundary(starting){modelReordering=starting;if(starting)rowOrderChanging();else rowOrderChanged()}
  signal rowOrderChanging()
  signal rowOrderChanged()
  function syncRows() {
    if (store.syncModel(rowsModel, sortKey, sortReverse,orderBoundary)) torrents = store.rows(store.modelOrder())
  }
  onSortKeyChanged: sortChanged()
  onSortReverseChanged: sortChanged()
  readonly property var activeTorrents: torrents.filter(Model.isActive)
  readonly property var otherTorrents: torrents.filter(function(t) { return !Model.isActive(t) })
  readonly property bool transferring: Number(status.dl) > 0 || Number(status.up) > 0
  property var status: ({ok:false, torrents:[]})
  property var store: Store.create()
  property var client: null
  property var widgets: []
  property var ipcOwner: null
  function registerWidget(widget) {
    if (widgets.indexOf(widget) < 0) widgets = widgets.concat([widget])
    if (!ipcOwner) ipcOwner = widget
  }
  function unregisterWidget(widget) {
    widgets = widgets.filter(function(w) { return w !== widget })
    if (ipcOwner === widget) {
      ipcOwner = null
      ipcClaim.restart()
    }
    updatePopup()
  }
  function updatePopup() { popupOpen = widgets.some(function(w) { return w.opened }) }


  function syncClient(c) {
    if (status.ok !== c.online) {
      var connectionStatus = Object.assign({}, status)
      connectionStatus.ok = c.online; connectionStatus.running = c.online
      status = connectionStatus
    }
    reconnecting = c.reconnecting; instance = c.instance
    if (c.online && startupRequested) { startupRequested=false; actionStatus=""; startupTimer.stop() }
    pending = c.state().pending
    resultUnknown=Object.keys(c.actions).some(function(ref){return c.actions[ref].status==="no answer"})
    store.syncActions(c.actions);actionRevision++
    var errorText=c.lastError ? c.lastError.message : ""
    if(errorText && errorText!==lastError)lastMessageAt=Date.now()
    lastError=errorText
    backendState = c.online && c.hello ? (store.summary.daemon ? store.summary.daemon.state : store.info.state || c.hello.state) : c.mode
  }
  function applyFrame(frame, request) {
    var rowsChanged
    try { rowsChanged = store.apply(frame, request) }
    catch (error) { client.reset("invalid state stream: " + String(error)); return }
    syncRows();badgeCounts=store.badges
    if(settingsFlow)settingsFlow.frame(frame)
    if(frame.topic==="settings" || (request && request.op.indexOf("app.settings.")===0))settingsRevision++
    if(clipboardFlow)clipboardFlow.frame(frame)
    if(addFlow)addFlow.frame(frame)
    if(detailFlow)detailFlow.frame(frame,request)
    if(overviewFlow)overviewFlow.frame(frame)
    if(cleanupFlow)cleanupFlow.frame(frame)
    if(searchFlow)searchFlow.frame(frame)
    if(frame.topic==="torrents" || (frame.topic && frame.topic.indexOf("torrent:")===0) || (request && request.op.indexOf("torrents.")===0))detailRevision++
    if(frame.topic==="log" || (request && request.op==="app.log"))logRevision++
    if (rowsChanged || frame.topic === "summary" || (request && frame.ok)) {
      status = Model.apiStatus(store, online)
    }
    if(request && !frame.ok && request.ref)notify(frame.error.message+" · Refresh to check the current state")
    if(frame.ev==="mutation.result" && frame.outcome!=="done")notify((frame.error ? frame.error.message : "No answer")+" · Refresh to check the current state")
    if (frame.ev === "bye") { byeReason=frame.reason; stoppedRecently = true; stoppedTimer.restart() }
  }
  function mutate(op, args, cap, callback) {
    if (!client) return false
    if (cap && client.caps.indexOf(cap) < 0) {
      lastError = "This daemon does not support " + cap
      actionStatus=lastError;noticeTimer.restart()
      return false
    }
    return client.request(op, args, {mutating:true, callback:callback})
  }
  function refresh() { if (client) client.subscribeAll() }
  function pause(hash) { return mutate("torrents.stop", {hashes:[hash]}) }
  function resume(hash) { return mutate("torrents.start", {hashes:[hash]}) }
  function toggleSeed(hash) {
    var row = store.torrents[hash]
    return row ? mutate("torrents.set_seed", {hashes:[hash], on:!row.seed}) : false
  }
  function remove(hash, deleteFiles) { return mutate("torrents.remove", {hashes:[hash], delete_files:!!deleteFiles}) }
  function add(source, stage) { var args={source:source}; if(stage)args.stage=true; return mutate("torrents.add", args, stage ? "staging" : "") }
  function commitStaged(hash, files, seed, savePath) { return mutate("torrents.commit", {hash:hash, files:files, seed:seed, save_path:savePath}, "staging") }
  function discardStaged(hash) { return mutate("torrents.discard", {hash:hash}, "staging") }
  function openTarget(hash, file) {
    var args = {hash:hash}
    if (file !== undefined) args.file = file
    return client.request("torrents.open_target", args, {callback:function(result) {
      if(result && result.path){
        var command=String(root.store.settings.open_command || "xdg-open")
        // Match Config::open: the owner-configured command is shell syntax;
        // the daemon-provided path remains a literal positional parameter.
        Commons.Util.execArgv(["sh","-c",'exec '+command+' "$1"',"fbittorrent-open",result.path])
      }
    }})
  }
  readonly property string diskBanner: {var confirmed=status;return online ? PopupState.diskBanner(store.summary.disk) : ""}
  readonly property bool turtleOn: {var confirmed=status;return store.summary.alt===true}
  function toggleTurtle(){return setTurtle(!turtleOn)}
  function setTurtle(on) { return mutate("transfer.set_alt", {on:on}, "limits") }
  function recheck(hash) { return mutate("torrents.recheck", {hashes:[hash]}, "recheck") }
  function quit() { return mutate("app.shutdown", {reason:"user"}) }
  function dismissClipboard(hash) { return mutate("app.clipboard.dismiss", {hash:hash}, "clipboard") }
  function cancelFocusTarget(target){if(windowFocus && windowFocus.target===target){windowFocus=null;focusRetry.stop()}}
  function cancelWindowFocus(window) {
    if(windowFocus && windowFocus.window===window){windowFocus=null;focusRetry.stop()}
  }
  function focusWindow(window,target) {
    if(!window || !target)return
    windowFocus={window:window,target:target,started:Date.now(),stage:0}
    windowFocusSteps=0
    advanceWindowFocus()
  }
  function advanceWindowFocus() {
    var request=windowFocus
    if(!request || !request.target || typeof request.target.forceActiveFocus!=="function" || !request.window || !request.window.visible){windowFocus=null;focusRetry.stop();return}
    request.target.forceActiveFocus()
    var backing=request.window._backingWindow,elapsed=Date.now()-request.started
    if(backing && backing.active && request.target.activeFocus){windowFocus=null;focusRetry.stop();return}
    if(request.stage===0) {
      request.target.forceActiveFocus()
      if(backing)backing.requestActivate()
      request.stage=1;windowFocusSteps++
    } else if(request.stage===1 && elapsed>=150) {
      var tops=ToplevelManager.toplevels.values
      for(var i=0;i<tops.length;i++)if(tops[i].title===request.window.title && tops[i].appId==="org.quickshell"){tops[i].activate();break}
      request.stage=2;windowFocusSteps++
    } else if(request.stage===2 && elapsed>=500 && Quickshell.env("HYPRLAND_INSTANCE_SIGNATURE")) {
      var windows=Hyprland.toplevels.values
      for(var j=0;j<windows.length;j++) {
        var data=windows[j].lastIpcObject || {}
        if(windows[j].title===request.window.title && data.class==="org.quickshell" && data.pid===Quickshell.processId) {
          var command=Focus.command(windows[j].address,Hyprland.usingLua)
          if(command)Hyprland.dispatch(command)
          break
        }
      }
      request.stage=3;windowFocusSteps++
    }
    if(elapsed>=1500){console.warn("fbittorrent dashboard focus timed out after 1500 ms");windowFocus=null;return}
    focusRetry.restart()
  }
  function startDaemon() {
    if (startupRequested) return
    startupRequested=true; actionStatus="Starting fbittorrent…"; noticeTimer.stop(); startupTimer.restart()
    Quickshell.execDetached([bin,"up"])
  }
  function openDashboardFilter(filter) { if(shell)shell.summon("oliver.torrents",JSON.stringify({view:"torrents",filter:filter})) }
  function openDashboard(view) {
    if (!shell || !shell.summon("oliver.torrents",JSON.stringify({view:view || "torrents"}))) {
      actionStatus="Dashboard is not available; update the plugin"; noticeTimer.restart()
    }
  }
  function acceptOffer() {
    var offer=clipboardOffer
    if(offer && mutate("torrents.add",{source:offer.source,origin:"clipboard"}))clipboardOffer=null
  }
  function cancelOffer() {
    var offer=clipboardOffer
    if(!offer || !offer.watched || dismissClipboard(offer.hash))clipboardOffer=null
  }
  function useRuntimeDir(path) {
    if (Quickshell.env("FBT_TEST_HOOKS") !== "1") return "refused: FBT_TEST_HOOKS=1 required"
    socket.connected = false
    runtimeOverride = path
    client.disconnected()
    Qt.callLater(function() { reconnect.restart() })
    return "ok"
  }
  function stateJson() { var state=client ? client.state() : {online:false,instance:"",subs:{},pending:[],view:null};state.view=dashboardState;return JSON.stringify(state) }
  function toggleTorrent(t) { var current=t ? store.torrents[t.hash] : null; return current ? (Model.isStopped(current) ? resume(current.hash) : pause(current.hash)) : false }
  function removeTorrent(t) { return t ? remove(t.hash, false) : false }
  function toggleSeeding(t) { return t ? toggleSeed(t.hash) : false }
  function openTorrent(t) { return t ? openTarget(t.hash) : false }
  function stopBackend() { return quit() }
  function startBackend() { startDaemon() }
  property bool clipboardTooLarge: false
  function addFromClipboard() {
    if (!clipboard.running) { clipboardTooLarge = false; clipboard.running = true }
  }
  function clipboardSource(text,allowDuplicate) {
    var parsed=AddSource.parse(text),source=parsed ? parsed.source : null,hash=parsed ? parsed.hash : ""
    if (hash && store.torrents[hash] && !allowDuplicate) { actionStatus = "Already in your list: " + store.torrents[hash].name; noticeTimer.restart(); return null }
    if (!source) { actionStatus = "Clipboard has no torrent"; noticeTimer.restart() }
    return source
  }
  onPopupOpenChanged: if (client) client.visibility(popupOpen, dashboardVisible, detailTopics.concat(addTopics,logTopics,searchTopics))
  onDashboardVisibleChanged: if (client) client.visibility(popupOpen, dashboardVisible, detailTopics.concat(addTopics,logTopics,searchTopics))
  onAddTopicsChanged:if(client)client.visibility(popupOpen,dashboardVisible,detailTopics.concat(addTopics,logTopics,searchTopics))
  onSearchTopicsChanged:if(client)client.visibility(popupOpen,dashboardVisible,detailTopics.concat(addTopics,logTopics,searchTopics))
  onLogTopicsChanged:if(client)client.visibility(popupOpen,dashboardVisible,detailTopics.concat(addTopics,logTopics,searchTopics))
  onDetailTopicsChanged: if (client) client.visibility(popupOpen, dashboardVisible, detailTopics.concat(addTopics,logTopics,searchTopics))

  FileView {
    id:preferences
    path:(Quickshell.env("XDG_STATE_HOME") || Quickshell.env("HOME")+"/.local/state")+"/fbittorrent/dashboard.json"
    atomicWrites:true;blockLoading:true;blockAllReads:true
    onLoaded:{
      try {
        var saved=JSON.parse(text());root.savedPreferences=saved;var sort=saved.sort || saved
        if(["added","name","progress","rate","size","state","eta","up","peers","ratio"].indexOf(sort.key)>=0)root.sortKey=sort.key
        root.sortReverse=sort.reverse===true
        if(Number.isFinite(saved.details_width))root.detailsWidth=Math.max(300,Math.min(480,saved.details_width))
      }catch(error){}
      root.preferencesLoaded=true
    }
    onLoadFailed:root.preferencesLoaded=true
  }
  Timer {id:savePreferences;interval:500;onTriggered:preferences.setText(JSON.stringify(Object.assign({},root.savedPreferences,{details_width:root.detailsWidth,sort:{key:root.sortKey,reverse:root.sortReverse}}))+"\n")}
  Timer {id:toastTimer;interval:5000;onTriggered:root.toast=null}
  Timer {interval:125;repeat:true;running:(root.dashboardVisible || root.popupOpen) && root.busy && !Commons.Style.reduceMotion;onTriggered:root.pendingFrame=(root.pendingFrame+1)%8}
  Socket {
    id: socket
    path: root.runtimeDir + "/fbittorrent/api.sock"
    parser: SplitParser { onRead: function(data) { if (root.client) root.client.receive(data) } }
    onConnectionStateChanged: {
      if (!root.client) return
      if (connected) { reconnect.stop(); root.client.connected() }
      else { root.client.disconnected(); reconnect.restart() }
    }
    onError: if (root.client && !connected) reconnect.restart()
  }
  Timer {
    id: reconnect
    interval: root.client ? root.client.retryDelay : 500
    onTriggered: {
      if (!root.client || socket.connected) return
      root.client.attempt()
      if (!socket.connected) restart()
    }
  }
  Timer { interval: 1000; repeat: true; running: !!root.client; onTriggered: { root.now=Date.now();root.client.tick(); root.syncRows();if(root.addFlow)root.addFlow.tick();if(root.settingsFlow)root.settingsFlow.tick() } }
  Timer { id:focusRetry;interval:50;onTriggered:root.advanceWindowFocus() }
  Timer { id: startupTimer; interval: 20000; onTriggered: {root.startupRequested=false;root.actionStatus="fbittorrent did not start; inspect the daemon log"} }
  Timer { id: noticeTimer; interval: 3000; onTriggered: root.actionStatus="" }
  Timer { id: ipcClaim; interval: 0; onTriggered: if (!root.ipcOwner && root.widgets.length) root.ipcOwner = root.widgets[0] }
  Timer { id: stoppedTimer; interval: 10000; onTriggered: root.stoppedRecently = false }
  Process {
    id: clipboard
    command: ["wl-paste", "--no-newline"]
    stdout: StdioCollector {
      waitForEnd: false
      onDataChanged: if (ApiClient.utf8Bytes(text) > 65536) {
        root.clipboardTooLarge = true
        root.lastError = "Clipboard exceeds 65536 bytes"
        clipboard.running = false
      }
      onStreamFinished: {
        if (root.clipboardTooLarge) return
        var sheet=root.clipboardForSheet;root.clipboardForSheet=false
        var source = root.clipboardSource(text,sheet)
        if(source){if(sheet)root.beginAdd(source);else root.clipboardOffer={source:source,name:Model.sourceName(source)}}
      }
    }
    onExited: function(code) { if (code !== 0) root.lastError = "Could not read clipboard" }
  }
  FileView {id:helpDocument;blockLoading:true;blockAllReads:true;preload:false}
  Component.onCompleted: {
    helpDocument.path=Qt.resolvedUrl("keymap.json")
    helpActions=JSON.parse(helpDocument.text()).actions.filter(function(row){return Array.isArray(row.keys.dashboard)})
    settingsFlow=SettingsFlow.create(root,function(){root.settingsRevision++})
    clipboardFlow=ClipboardFlow.create(root)
    browsePosters=BrowsePosters.create(root,function(){root.browsePosterRevision++})
    searchFlow=SearchFlow.create(root,function(){root.searchRevision++})
    detailFlow=DetailFlow.create(root,function(){root.detailRevision++})
    overviewFlow=OverviewFlow.create(root,function(){root.overviewRevision++})
    cleanupFlow=CleanupFlow.create(root,function(){root.cleanupRevision++})
    addFlow=AddFlow.create(root,function(){root.addRevision++},Quickshell.env("FBT_TEST_HOOKS")==="1")
    client = ApiClient.create({
      write:function(text) { socket.write(text) }, flush:function() { socket.flush() },
      isConnected:function() { return socket.connected }, connect:function() { socket.connected = true },
      disconnect:function() { socket.connected = false }
    }, {version:manifest.version || "0.2.0", changed:syncClient, frame:applyFrame,
      instance:function() { if(root.cleanupFlow)root.cleanupFlow.reset();if(root.overviewFlow)root.overviewFlow.instanceChanged();if(root.clipboardFlow)root.clipboardFlow.reset();root.store = Store.create(); root.torrents = []; if(root.detailFlow)root.detailFlow.reset() }})
    client.visibility(popupOpen,dashboardVisible,detailTopics.concat(addTopics,logTopics,searchTopics))
    if (socket.connected) client.connected()
    else { client.attempt(); if (socket.connected) client.connected() }
    if (!socket.connected) reconnect.start()
  }
}
