import QtQuick

// Launcher / command palette. In phase C its catalogue is the real capability registry,
// filtered by permission, and it never searches the web while the person types.
Panel {
    fill: Theme.solid
    id: pal
    width: 640; height: 372
    property int current: 0
    Rectangle { x: 16; y: 16; width: parent.width - 32; height: 44; radius: Theme.radius; color: Theme.raised; border.width: 1; border.color: Theme.accent
        Row { x: 14; anchors.verticalCenter: parent.verticalCenter; spacing: 10
            Text { text: "⌕"; color: Theme.accent; font.pixelSize: Theme.fsTitle }
            Text { text: "vault"; color: Theme.ink; font.pixelSize: Theme.fsTitle }
            Rectangle { width: 2; height: 20; color: Theme.accent; anchors.verticalCenter: parent.verticalCenter }
        } }
    Column { x: 16; y: 72; width: parent.width - 32; spacing: 2
        Repeater { model: Synthetic.results
            delegate: Rectangle { width: parent.width; height: 44; radius: 8
                color: pal.current === index ? Qt.rgba(Theme.accent.r, Theme.accent.g, Theme.accent.b, 0.22) : "transparent"
                Row { x: 12; anchors.verticalCenter: parent.verticalCenter; spacing: 12
                    Chip { text: modelData.kind; tone: modelData.kind === "Acción" ? Theme.warn : Theme.unknown }
                    Column { spacing: 1
                        Text { text: modelData.name; color: Theme.ink; font.pixelSize: Theme.fsBody }
                        Text { text: modelData.hint; color: Theme.dim; font.pixelSize: Theme.fsSmall }
                    }
                }
                MouseArea { anchors.fill: parent; hoverEnabled: true; onEntered: pal.current = index }
            } }
    }
    Row { x: 16; y: parent.height - 28; spacing: 16
        Text { text: "↑↓ elegir"; color: Theme.dim; font.pixelSize: Theme.fsSmall }
        Text { text: "↵ abrir"; color: Theme.dim; font.pixelSize: Theme.fsSmall }
        Text { text: "esc cerrar"; color: Theme.dim; font.pixelSize: Theme.fsSmall }
        DesignBanner {}
    }
}
