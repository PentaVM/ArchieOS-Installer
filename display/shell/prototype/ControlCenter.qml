import QtQuick

Panel {
    fill: Theme.solid
    id: cc
    width: 380; height: 470
    Column { x: 16; y: 16; width: parent.width - 32; spacing: 12
        Text { text: "Centro de control"; color: Theme.ink; font.pixelSize: Theme.fsTitle; font.bold: true }
        Grid { columns: 2; spacing: 8; width: parent.width
            Repeater { model: Synthetic.quick
                delegate: Rectangle { width: (parent.width - 8) / 2; height: 62; radius: Theme.radius
                    color: modelData.on ? Qt.rgba(Theme.accent.r, Theme.accent.g, Theme.accent.b, 0.20) : Theme.raised
                    border.width: 1; border.color: modelData.on ? Theme.accent : Theme.line
                    Column { x: 12; anchors.verticalCenter: parent.verticalCenter; spacing: 2
                        Text { text: modelData.name; color: Theme.ink; font.pixelSize: Theme.fsBody; font.bold: true }
                        Text { text: modelData.state; color: Theme.soft; font.pixelSize: Theme.fsSmall }
                    }
                } }
        }
        // Gauges: an unmeasured value draws a dashed rule, never a bar at zero (RFC-006).
        Text { text: "RECURSOS"; color: Theme.dim; font.pixelSize: Theme.fsSmall - 1; font.letterSpacing: 1.2 }
        Repeater { model: [ { n: "Presión CPU (PSI)", v: 0.22, m: true }, { n: "GPU / VRAM", v: 0.0, m: false }, { n: "Temperatura NVMe", v: 0.0, m: false } ]
            delegate: Column { width: cc.width - 32; spacing: 3
                Row { spacing: 6; Text { text: modelData.n; color: Theme.ink; font.pixelSize: Theme.fsSmall }
                    Text { visible: !modelData.m; text: "no medido: sin lectura"; color: Theme.dim; font.pixelSize: Theme.fsSmall } }
                Item { width: parent.width; height: 8
                    Rectangle { visible: modelData.m; anchors.fill: parent; radius: 4; color: Theme.raised
                        Rectangle { width: parent.width * modelData.v; height: parent.height; radius: 4; color: Theme.ok } }
                    Row { visible: !modelData.m; spacing: 4; anchors.verticalCenter: parent.verticalCenter
                        Repeater { model: 26; Rectangle { width: 8; height: 2; color: Theme.dim } } }
                }
            } }
        Rectangle { width: parent.width; height: 1; color: Theme.line }
        // Emergency pause: disabled on purpose, because no IPC exists yet (GUI-POLICY-002).
        Rectangle { width: parent.width; height: 48; radius: Theme.radius; color: "transparent"; border.width: 1; border.color: Theme.line
            Column { x: 12; anchors.verticalCenter: parent.verticalCenter; spacing: 1
                Text { text: "Ⅱ  Pausa de emergencia de autoridad"; color: Theme.dim; font.pixelSize: Theme.fsBody }
                Text { text: "No disponible: archied no expone este control todavía"; color: Theme.warn; font.pixelSize: Theme.fsSmall }
            } }
        DesignBanner {}
    }
}
