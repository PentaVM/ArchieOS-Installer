import QtQuick

// Error line: the real reason, red icon and word (never colour alone).
Rectangle {
    id: e
    property string text
    width: parent ? parent.width : 300; height: t.implicitHeight + 18
    radius: 8; color: Theme.tint(Theme.bad, 0.10); border.width: 1; border.color: Theme.tint(Theme.bad, 0.30)
    Row { x: 10; y: 9; spacing: 8; width: parent.width - 20
        Icon { name: "bad"; size: 14; color: Theme.bad }
        Text { id: t; width: parent.width - 22; text: e.text; wrapMode: Text.Wrap; color: Theme.bad
            font.family: Theme.font; font.pixelSize: 12; lineHeight: 17; lineHeightMode: Text.FixedHeight } }
}
