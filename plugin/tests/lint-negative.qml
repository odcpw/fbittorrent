// Deliberately invalid token use, never loaded as a UI. One occurrence per rule.
Item {
  Rectangle { color: "#ff0000" }
  Rectangle { color: Qt.rgba(1, 0, 0, 1) }
  Rectangle { color: "white" }
  Text { font.pixelSize: 12 }
  NumberAnimation { duration: 400 }
  property string listOperation: "torrents.list"
  property string filesOperation: "torrents.files"
}
