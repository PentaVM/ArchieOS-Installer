// A window that says when input reached it: every key and pointer event, with the wall clock
// in milliseconds, one JSON line each on stdout. The Stage latency probe compares these with
// when it sent each action.
import QtQuick
import QtQuick.Window

Window {
    width: 800; height: 600; visible: true; title: "archie latency probe"
    color: "white"
    function log(kind, extra) { console.log(JSON.stringify(Object.assign({t: Date.now(), e: kind}, extra))) }
    Item {
        anchors.fill: parent; focus: true
        Keys.onPressed: (event) => { if (!event.isAutoRepeat) log("key_down", {key: event.key}) }
        Keys.onReleased: (event) => { if (!event.isAutoRepeat) log("key_up", {key: event.key}) }
        MouseArea {
            anchors.fill: parent; hoverEnabled: true; acceptedButtons: Qt.AllButtons
            onPressed: (mouse) => log("button_down", {x: mouse.x, y: mouse.y})
            onReleased: (mouse) => log("button_up", {x: mouse.x, y: mouse.y})
            onPositionChanged: (mouse) => log("motion", {x: mouse.x, y: mouse.y})
        }
    }
}
