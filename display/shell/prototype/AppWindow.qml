import QtQuick

// Window with title bar (traffic lights, centred title, trailing slot), a 200 px sidebar and a
// main area. Used by Forge and Settings. With `wid` set, the lights close, minimise and zoom it
// and the title bar drags it (see Wm); without it they are only drawn.
Panel {
    id: w
    kind: "window"
    property string title
    property string wid: ""
    default property alias main: mainCol.data
    property alias sidebar: sideCol.data
    property alias trailing: trail.data
    enterScale: 0.96
    durIn: Theme.durOpen; durOut: Theme.durClose

    Item { id: tbar; width: parent.width; height: 52
        Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.hair }
        TitleDrag { wid: w.wid; enabled: w.wid !== "" }
        Row { x: 16; anchors.verticalCenter: parent.verticalCenter; spacing: 8; visible: w.wid === ""
            Rectangle { width: 12; height: 12; radius: 6; color: Theme.tlClose; border.width: 1; border.color: Qt.rgba(0, 0, 0, 0.22) }
            Rectangle { width: 12; height: 12; radius: 6; color: Theme.tlMin; border.width: 1; border.color: Qt.rgba(0, 0, 0, 0.22) }
            Rectangle { width: 12; height: 12; radius: 6; color: Theme.tlZoom; border.width: 1; border.color: Qt.rgba(0, 0, 0, 0.22) } }
        Lights { x: 16; anchors.verticalCenter: parent.verticalCenter; wid: w.wid; visible: w.wid !== "" }
        Text { anchors.centerIn: parent; text: w.title; color: Theme.t1; font.family: Theme.disp; font.pixelSize: 17; font.weight: Font.DemiBold; font.letterSpacing: -0.17 }
        Row { id: trail; anchors.right: parent.right; anchors.rightMargin: 16; anchors.verticalCenter: parent.verticalCenter; spacing: 8 } }
    Rectangle { id: side; y: tbar.height; width: 200; height: parent.height - tbar.height - 1; color: Theme.side
        Rectangle { anchors.right: parent.right; width: 1; height: parent.height; color: Theme.hair }
        Column { id: sideCol; x: 10; y: 14; width: parent.width - 20; spacing: 2 } }
    Column { id: mainCol; x: side.width + 20; y: tbar.height + 18; width: parent.width - side.width - 40; spacing: 12 }
}
