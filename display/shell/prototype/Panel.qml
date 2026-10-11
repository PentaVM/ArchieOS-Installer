import QtQuick

// A shell surface: translucent fill, hairline border, soft shadow made of two stacked rects
// (no shader needed, so it costs nothing on the GPU that IRIS is using).
Item {
    id: p
    property alias contentItem: holder
    default property alias data: holder.data
    property int radius: Theme.radius
    property bool elevated: true
    property color fill: Theme.surface

    Rectangle {
        visible: p.elevated
        anchors.fill: parent; anchors.margins: -1; anchors.topMargin: 4; anchors.bottomMargin: -6
        radius: p.radius + 2; color: "#000000"; opacity: 0.28
    }
    Rectangle {
        anchors.fill: parent; radius: p.radius; color: p.fill
        border.width: 1; border.color: Theme.line
    }
    Item { id: holder; anchors.fill: parent }
}
