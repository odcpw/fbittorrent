import QtQuick
import Quickshell.Io

// Own each native connection separately. A failed QLocalSocket can remain
// allocated after an error, and disconnectFromServer can wait for buffered
// writes. Replacing the object retires both its transport and parser state.
Loader {
  id:root
  objectName:"api-owner-socket"
  required property var service
  required property var retry
  required property var handshake
  property bool recovering:false
  property int delay:1000
  property int generation:0
  property bool resetting:false
  property bool wasOnline:false
  property bool reachedTransport:false
  readonly property bool connected:!resetting && !!item && item.connected
  readonly property string path:service.runtimeDir+"/fbittorrent/api.sock"

  function endRecovery(){recovering=false;retry.stop();handshake.stop();delay=1000}
  function write(text){if(item && !resetting)item.write(text)}
  function flush(){if(item && !resetting)item.flush()}
  function connectToOwner(){if(item && !resetting){handshake.restart();item.connected=true}}
  function replace(recover,openAfter){
    handshake.stop()
    recovering=!!recover
    if(recover)retry.restart();else endRecovery()
    resetting=true;wasOnline=false;reachedTransport=false
    var epoch=++generation
    // Leave native signal handlers before deleting their sender. A second
    // event turn ensures the old object is retired before opening its successor.
    Qt.callLater(function(){
      if(epoch!==root.generation)return
      root.active=false
      Qt.callLater(function(){
        if(epoch!==root.generation)return
        root.active=true;root.resetting=false
        if(openAfter)root.service.checkSocket(true)
      })
    })
  }
  function retryExpired(){
    if(!recovering)return
    service.checkSocket(true);delay=Math.min(delay*2,15000);retry.restart()
  }
  function helloExpired(){
    if(!service.client || service.client.online || service.client.graceful || service.client.protocolMismatch)return
    replace(true);service.client.disconnected()
  }
  sourceComponent:Socket {
    id:connection
    property int epoch:0
    Component.onCompleted:epoch=root.generation
    path:root.path
    parser:SplitParser {onRead:function(data){
      if(connection.epoch===root.generation && root.item===connection && root.service.client)
        root.service.client.receive(data)
    }}
    onConnectionStateChanged:{
      if(connection.epoch!==root.generation || root.item!==connection || !root.service.client)return
      if(connected){
        root.wasOnline=false;root.reachedTransport=true;handshake.restart();root.service.client.connected()
      }else{
        var recover=(root.recovering || root.wasOnline || root.reachedTransport) && !root.service.client.graceful && !root.service.client.protocolMismatch
        root.replace(recover);root.service.client.disconnected()
      }
    }
    onError:function(error){
      if(connection.epoch!==root.generation || root.item!==connection || !root.service.client)return
      // QLocalSocket: refused=0, server not found=2. Those prove the
      // listener is gone; a full backlog and peer closure do not.
      var recover=error!==0 && error!==2 && !root.service.client.graceful && !root.service.client.protocolMismatch
      root.replace(recover);root.service.client.disconnected()
    }
  }
}
