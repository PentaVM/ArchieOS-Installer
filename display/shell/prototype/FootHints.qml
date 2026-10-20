import QtQuick

// Foot of a panel: hairline, key hints on the left, a note on the right.
// `hints` is a list of [key, word] pairs. Width is the parent's.
Item {
    id: f
    property var hints: []
    property string note
    width: parent ? parent.width : 300; height: 38
    Rectangle { width: parent.width; height: 1; color: Theme.hair }
    Row { x: 20; anchors.verticalCenter: parent.verticalCenter; spacing: 18
        Repeater { model: f.hints
            delegate: Row { spacing: 4
                Kbd { label: modelData[0]; anchors.verticalCenter: parent.verticalCenter }
                Text { text: modelData[1]; color: Theme.t3; font.family: Theme.font; font.pixelSize: 12; leftPadding: 2; anchors.verticalCenter: parent.verticalCenter } } } }
    Text { visible: f.note !== ""; anchors.right: parent.right; anchors.rightMargin: 20; anchors.verticalCenter: parent.verticalCenter
        text: f.note; color: Theme.t3; font.family: Theme.font; font.pixelSize: 12 }
}
