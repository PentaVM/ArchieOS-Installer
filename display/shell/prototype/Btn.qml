import QtQuick

// Button. `dis` = disabled: dashed, dim; the reason goes in a Why next to it.
Item {
    id: b
    property string text
    property bool dis: false
    property bool ring: false          // visible-focus ring
    signal clicked()
    implicitWidth: lbl.implicitWidth + 26; implicitHeight: 28
    width: implicitWidth; height: implicitHeight
    Rectangle { anchors.fill: parent; radius: 8; visible: !b.dis
        color: ma.containsMouse ? Theme.pressed : Theme.hover; border.width: 1; border.color: Theme.hair }
    DashedRect { anchors.fill: parent; radius: 8; visible: b.dis; color: Theme.hair }
    Text { id: lbl; anchors.centerIn: parent; text: b.text; color: b.dis ? Theme.tdis : Theme.t1
        font.family: Theme.font; font.pixelSize: 12; font.weight: Font.Medium }
    Rectangle { anchors.fill: parent; anchors.margins: -4; radius: 11; color: "transparent"; border.width: 2; border.color: Theme.acc; visible: b.ring }
    MouseArea { id: ma; anchors.fill: parent; hoverEnabled: true; enabled: !b.dis; onClicked: b.clicked() }
}
