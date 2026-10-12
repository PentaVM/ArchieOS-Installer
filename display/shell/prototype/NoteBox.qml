import QtQuick

// Dashed note with an icon: says what the surface does not do, and why.
Item {
    id: n
    property string text
    property string icon: "lock"
    width: parent ? parent.width : 300; height: t.implicitHeight + 18
    DashedRect { anchors.fill: parent; radius: 8; color: Theme.hair }
    Row { x: 12; y: 9; spacing: 8; width: parent.width - 24
        Icon { name: n.icon; size: 14; color: Theme.t3; y: 1 }
        Text { id: t; width: parent.width - 22; text: n.text; wrapMode: Text.Wrap; color: Theme.t3
            font.family: Theme.font; font.pixelSize: 12; lineHeight: 16; lineHeightMode: Text.FixedHeight } }
}
