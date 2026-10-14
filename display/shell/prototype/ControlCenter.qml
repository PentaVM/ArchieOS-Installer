import QtQuick

// Control center, 440 px wide. REAL sections come from archie-shell-bridge (archie-displayd);
// DISEÑO sections are placeholders with no service behind them.
Panel {
    id: cc
    kind: "panel"
    property var bridge
    width: 440
    height: content.implicitHeight + 40
    readonly property var sys: bridge.system

    Column { id: content; x: 20; y: 20; width: parent.width - 40; spacing: 0
        Text { text: "Centro de control"; color: Theme.t1; font.family: Theme.disp; font.pixelSize: 22; font.weight: Font.DemiBold; font.letterSpacing: -0.33 }
        Flow { width: parent.width; topPadding: 12; spacing: 8
            Chip { src: true; icon: "ok"; text: "Real · bridge" }
            LinkChip { bridge: cc.bridge } }

        // Failure box: the real reason, and why the summary is hidden.
        Rectangle { visible: !sys && bridge.reason !== ""; width: parent.width; height: fail.implicitHeight + 24; radius: Theme.rCard
            color: Theme.tint(Theme.bad, 0.09); border.width: 1; border.color: Theme.tint(Theme.bad, 0.32)
            Row { id: fail; x: 14; y: 12; spacing: 12; width: parent.width - 28
                Icon { name: "bad"; size: 18; color: Theme.bad }
                Column { width: parent.width - 30; spacing: 4
                    Text { text: "No se puede leer el estado del sistema"; color: Theme.t1; font.family: Theme.font; font.pixelSize: 13; font.weight: Font.DemiBold }
                    Text { width: parent.width; text: bridge.reason; wrapMode: Text.Wrap; maximumLineCount: 3; elide: Text.ElideRight
                        color: Theme.t2; font.family: Theme.font; font.pixelSize: 12; lineHeight: 17; lineHeightMode: Text.FixedHeight }
                    Text { width: parent.width; wrapMode: Text.Wrap; text: "Resumen del sistema oculto: solo se muestra si el daemon respondió."; color: Theme.t3; font.family: Theme.font; font.pixelSize: 11 } } } }
        Item { visible: !sys && bridge.reason !== ""; width: 1; height: 16 }

        // System summary: only when archied answered.
        Item { visible: sys !== null; width: 1; height: 16 }
        Rectangle { visible: sys !== null; width: parent.width; height: kv.implicitHeight + 28; radius: Theme.rCard
            color: Theme.raised; border.width: 1; border.color: Theme.hair
            Grid { id: kv; x: 14; y: 14; columns: 3; columnSpacing: 16; rowSpacing: 12; width: parent.width - 28
                Repeater { model: sys ? [
                    ["Equipo", sys.hostname], ["Versión", sys.archieos_version], ["Salud", sys.health],
                    ["Módulos", sys.modules_loaded + " / " + sys.modules_installed], ["Modelos", String(sys.models_registered)],
                    ["Tareas activas", String(sys.active_tasks)] ] : []
                    delegate: Column { width: (kv.width - 32) / 3; spacing: 2
                        Text { text: modelData[0]; color: Theme.t3; font.family: Theme.font; font.pixelSize: 11 }
                        Row { spacing: 6
                            Icon { visible: modelData[0] === "Salud" && modelData[1] === "healthy"; name: "done"; size: 14; color: Theme.ok; anchors.verticalCenter: parent.verticalCenter }
                            Text { text: modelData[1]; color: Theme.t1; font.family: Theme.font; font.pixelSize: 13; font.weight: Font.Medium; elide: Text.ElideRight; width: parent.parent.width - (modelData[0] === "Salud" && modelData[1] === "healthy" ? 20 : 0) } } } } } }

        Item { width: 1; height: 22 }
        ResourceGauges { bridge: cc.bridge; width: parent.width }

        Rectangle { width: parent.width; height: 1; color: Theme.hair; y: 0 }
        Item { width: 1; height: 18 }
        Chip { design: true; icon: "sample"; text: "Diseño · sin servicio" }
        Item { width: 1; height: 12 }
        // quick tiles: no service is connected, so each is dashed and says so. The exception is Silencio:
        // it only governs this shell's own notifications, so it works, and says it is local.
        Grid { columns: 2; spacing: 10; width: parent.width
            Repeater { model: Synthetic.quick
                delegate: Item { id: tile
                    readonly property bool live: modelData.name === "Silencio"
                    readonly property bool on: live && Demo.silent
                    width: (content.width - 10) / 2; height: 62
                    Rectangle { visible: tile.live; anchors.fill: parent; radius: Theme.rCard; color: tile.on ? Theme.sel : (tma.containsMouse ? Theme.hover : Theme.raised)
                        border.width: 1; border.color: tile.on ? Theme.tint(Theme.acc, 0.45) : Theme.hair }
                    DashedRect { visible: !tile.live; anchors.fill: parent; radius: Theme.rCard; color: Theme.hair }
                    Column { x: 12; y: 12; spacing: 6
                        Row { spacing: 8; Icon { name: modelData.icon; size: 20; color: tile.on ? Theme.acc : (tile.live ? Theme.t1 : Theme.t3); anchors.verticalCenter: parent.verticalCenter }
                            Text { text: modelData.name; color: tile.live ? Theme.t1 : Theme.t2; font.family: Theme.font; font.pixelSize: 13; font.weight: Font.Medium; anchors.verticalCenter: parent.verticalCenter } }
                        Row { spacing: 5; Icon { name: tile.live ? (tile.on ? "done" : "unknown") : "lock"; size: 12; color: tile.on ? Theme.acc : Theme.t3; anchors.verticalCenter: parent.verticalCenter }
                            Text { text: tile.live ? (tile.on ? "activado · solo local" : "desactivado · solo local") : "sin servicio conectado"
                                color: Theme.t3; font.family: Theme.font; font.pixelSize: 11; anchors.verticalCenter: parent.verticalCenter } } }
                    MouseArea { id: tma; anchors.fill: parent; hoverEnabled: true; enabled: tile.live; onClicked: Demo.silent = !Demo.silent } } } }

        // Emergency pause: disabled on purpose, no IPC exists yet (GUI-POLICY-002).
        Item { width: 1; height: 12 }
        Item { width: parent.width; height: emg.implicitHeight + 28
            DashedRect { anchors.fill: parent; radius: Theme.rCard; color: Theme.tint(Theme.warn, 0.45) }
            Row { id: emg; x: 14; y: 14; spacing: 12
                Icon { name: "pause"; size: 20; color: Theme.t3 }
                Column { spacing: 3
                    Text { text: "Pausa de emergencia de autoridad"; color: Theme.t2; font.family: Theme.font; font.pixelSize: 13; font.weight: Font.Medium }
                    Text { text: "Deshabilitado: archied no expone este control todavía"; color: Theme.warn; font.family: Theme.font; font.pixelSize: 11 } } } }
    }
}
