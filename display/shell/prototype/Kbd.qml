import QtQuick

// Key cap.
Rectangle {
    property string label
    implicitWidth: Math.max(20, kt.implicitWidth + 12); implicitHeight: 20; radius: 5
    color: Theme.raised; border.width: 1; border.color: Theme.hair
    Text { id: kt; anchors.centerIn: parent; text: parent.label; color: Theme.t2; font.family: Theme.font; font.pixelSize: 11; font.weight: Font.Medium }
}
