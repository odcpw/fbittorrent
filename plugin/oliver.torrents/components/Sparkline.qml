import QtQuick
import qs.Commons as Commons

// One bar per received sample. Paint only on data, geometry or palette changes.
Canvas {
  id:root
  property var samples:[]
  property color foreground:Commons.Color.foreground
  readonly property var points:Array.isArray(samples) ? samples.slice(-60) : []
  readonly property real maximum:Math.max(1,...points)
  implicitWidth:180
  implicitHeight:24
  Accessible.name:"Download speed history"
  onPointsChanged:requestPaint()
  onForegroundChanged:requestPaint()
  onWidthChanged:requestPaint()
  onHeightChanged:requestPaint()
  onVisibleChanged:if(visible)requestPaint()
  onPaint:{
    var ctx=getContext("2d");ctx.clearRect(0,0,width,height)
    if(!points.length || width<=0 || height<=0)return
    ctx.fillStyle=foreground
    var step=width/points.length
    for(var i=0;i<points.length;i++){
      var value=Math.max(0,Number(points[i])),h=Math.round(height*value/maximum)
      ctx.fillRect(Math.floor(i*step),height-h,Math.max(1,Math.floor(step)-1),h)
    }
  }
}
