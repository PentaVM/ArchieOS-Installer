import QtQuick

// Control center. Sections say where their data comes from: REAL = archie-shell-bridge
// (archie-displayd), DISEÑO = synthetic placeholder with no service behind it yet.
Panel {
    id: cc
    property var bridge
    fill: Theme.solid
    width: 400; height: 640
    Column { x: 16; y: 16; width: parent.width - 32; spacing: 12
        Text { text: "Centro de control"; color: Theme.ink; font.pixelSize: Theme.fsTitle; font.bold: true }

        // Real: link state and, when archied answered, its status (GUI-CORE-001, GUI-CORE-013).
        Row { spacing: 8
            Chip { text: "REAL · bridge"; tone: Theme.info }
            LinkChip { bridge: cc.bridge } }
        Column { visible: cc.bridge.system !== null; spacing: 2
            Text { text: cc.bridge.system ? cc.bridge.system.hostname + " · ArchieOS " + cc.bridge.system.archieos_version : ""; color: Theme.ink; font.pixelSize: Theme.fsBody }
            Text { text: cc.bridge.system ? "salud: " + cc.bridge.system.health + " · módulos " + cc.bridge.system.modules_loaded + "/" + cc.bridge.system.modules_installed
                    + " · modelos " + cc.bridge.system.models_registered + " · tareas activas " + cc.bridge.system.active_tasks : ""
                color: Theme.soft; font.pixelSize: Theme.fsSmall } }
        Text { visible: cc.bridge.reason !== ""; width: parent.width; wrapMode: Text.Wrap; text: cc.bridge.reason
            color: Theme.dim; font.pixelSize: Theme.fsSmall - 1; maximumLineCount: 3; elide: Text.ElideRight }

        ResourceGauges { bridge: cc.bridge; width: parent.width }

        Rectangle { width: parent.width; height: 1; color: Theme.line }

        // Synthetic: no Wi-Fi / Bluetooth / audio service is connected yet.
        Row { spacing: 8
            Chip { text: "DISEÑO · sin servicio"; tone: Theme.warn } }
        Grid { columns: 2; spacing: 8; width: parent.width
            Repeater { model: Synthetic.quick
                delegate: Rectangle { width: (parent.width - 8) / 2; height: 54; radius: Theme.radius
                    color: Theme.raised; border.width: 1; border.color: Theme.line
                    Column { x: 12; anchors.verticalCenter: parent.verticalCenter; spacing: 2
                        Text { text: modelData.name; color: Theme.dim; font.pixelSize: Theme.fsBody; font.bold: true }
                        Text { text: "sin servicio conectado"; color: Theme.dim; font.pixelSize: Theme.fsSmall }
                    }
                } }
        }
        // Emergency pause: disabled on purpose, because no IPC exists yet (GUI-POLICY-002).
        Rectangle { width: parent.width; height: 48; radius: Theme.radius; color: "transparent"; border.width: 1; border.color: Theme.line
            Column { x: 12; anchors.verticalCenter: parent.verticalCenter; spacing: 1
                Text { text: "Ⅱ  Pausa de emergencia de autoridad"; color: Theme.dim; font.pixelSize: Theme.fsBody }
                Text { text: "No disponible: archied no expone este control todavía"; color: Theme.warn; font.pixelSize: Theme.fsSmall }
            } }
    }
}
