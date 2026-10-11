import QtQuick

Panel {
    id: dock
    width: row.implicitWidth + 2 * Theme.gap
    height: Theme.dockH + 2 * 6
    radius: Theme.radius + 4
    property string current: "vault"
    signal launched(string id)

    Row {
        id: row; anchors.centerIn: parent; spacing: Theme.gap
        Repeater {
            model: Synthetic.dock
            delegate: Item {
                id: cell
                width: Theme.dockH - 8; height: Theme.dockH - 8
                property bool hot: hover.containsMouse
                scale: hot ? 1.12 : 1.0
                Behavior on scale { NumberAnimation { duration: Theme.fast; easing.type: Easing.OutCubic } }
                Rectangle {
                    anchors.fill: parent; radius: Theme.radius
                    gradient: Gradient {
                        GradientStop { position: 0; color: Qt.lighter(modelData.color, 1.15) }
                        GradientStop { position: 1; color: Qt.darker(modelData.color, 1.7) }
                    }
                    Text { anchors.centerIn: parent; text: modelData.glyph; color: "#0b0e14"; font.pixelSize: parent.height * 0.5; font.bold: true }
                }
                // Running dot: a verified process, not decoration.
                Rectangle { visible: modelData.running; width: 5; height: 5; radius: 3; color: Theme.ink
                    anchors.horizontalCenter: parent.horizontalCenter; anchors.top: parent.bottom; anchors.topMargin: 3 }
                Rectangle { visible: dock.current === modelData.id; anchors.fill: parent; radius: Theme.radius; color: "transparent"
                    border.width: 2; border.color: Theme.accent }
                Rectangle { visible: hover.containsMouse; y: -26; anchors.horizontalCenter: parent.horizontalCenter
                    width: tip.implicitWidth + 14; height: 20; radius: 5; color: Theme.solid; border.width: 1; border.color: Theme.line
                    Text { id: tip; anchors.centerIn: parent; text: modelData.name; color: Theme.ink; font.pixelSize: Theme.fsSmall } }
                MouseArea { id: hover; anchors.fill: parent; hoverEnabled: true; onClicked: dock.launched(modelData.id) }
            }
        }
    }
}
