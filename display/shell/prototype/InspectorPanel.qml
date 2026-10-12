import QtQuick

// IRIS / Cortex inspector, 540 px. Debug mode only. DESIGN SAMPLE. Cortex trajectories arrive
// already minimised for privacy (their content is withdrawn, so bars stand in for it). IRIS has
// no read API: its internal state is never drawn, no thoughts, no telemetry.
Panel {
    id: ip
    kind: "panel"
    width: 540
    height: foot.y + foot.height
    property string st: Demo.state

    component Traj: Item {
        id: tj
        property string name
        property string time
        property bool ring: false
        property bool unread: false
        property string detail
        width: parent ? parent.width : 400
        height: tb.implicitHeight + 20
        Rectangle { anchors.fill: parent; radius: 10; color: tj.ring ? Theme.sel : "transparent"; border.width: tj.ring ? 2 : 0; border.color: Theme.acc }
        Rectangle { x: 10; y: 10; width: 32; height: 32; radius: 8; color: Qt.rgba(1, 1, 1, 0.05); border.width: 1; border.color: Theme.hair
            Icon { anchors.centerIn: parent; name: tj.unread ? "unknown" : "trace"; size: 16; color: Theme.t2 } }
        Column { id: tb; x: 54; y: 10; width: parent.width - 54 - 84; spacing: 6
            Text { width: parent.width; elide: Text.ElideRight; text: tj.name; color: Theme.t1; font.family: tj.unread ? Theme.font : "monospace"; font.pixelSize: tj.unread ? 13 : 12; font.weight: Font.Medium }
            Text { visible: tj.unread; width: parent.width; text: tj.detail; color: Theme.t2; font.family: Theme.font; font.pixelSize: 12 }
            Column { visible: !tj.unread; spacing: 5; width: parent.width
                Rectangle { width: parent.width * 0.92; height: 6; radius: 3; color: Qt.rgba(185 / 255, 184 / 255, 196 / 255, 0.24) }
                Rectangle { width: parent.width * 0.64; height: 6; radius: 3; color: Qt.rgba(185 / 255, 184 / 255, 196 / 255, 0.24) } }
            Row { spacing: 8
                Chip { small: true; tone: tj.unread ? "unk" : "neutral"; icon: tj.unread ? "unknown" : "lines"; text: tj.unread ? "no medido" : "minimizada" }
                Text { visible: !tj.unread; text: "contenido retirado por privacidad"; color: Theme.t3; font.family: Theme.font; font.pixelSize: 11; anchors.verticalCenter: parent.verticalCenter } } }
        Text { anchors.right: parent.right; anchors.rightMargin: 10; y: 10; text: tj.time; color: Theme.t3; font.family: "monospace"; font.pixelSize: 11 }
    }

    Column { id: col; x: 20; y: 20; width: parent.width - 40; spacing: 0
        Item { width: parent.width; height: 24
            Eyebrow { text: "Inspector"; anchors.verticalCenter: parent.verticalCenter }
            Chip { icon: "bug"; tone: "warn"; text: "Modo depuración"; anchors.right: parent.right; anchors.verticalCenter: parent.verticalCenter } }
        Item { width: 1; height: 10 }
        Text { text: "IRIS · Cortex"; color: Theme.t1; font.family: Theme.disp; font.pixelSize: 22; font.weight: Font.DemiBold; font.letterSpacing: -0.33 }
        Flow { width: parent.width; topPadding: 12; bottomPadding: 4; spacing: 8
            Chip { design: true; icon: "sample"; text: "Diseño · datos de ejemplo" }
            StateChip { st: ip.st; noun: "trayectorias" } }
        Item { width: 1; height: 8 }
        ErrBox { visible: ip.st === "error"; text: "No se pudieron leer las trayectorias de Cortex: respuesta no válida." }
        VoidBox { visible: ip.st === "vacio"; icon: "trace"; title: "Sin trayectorias registradas"; text: "Cortex no ha guardado ninguna trayectoria en esta sesión." }
        Column { visible: ip.st === "normal" || ip.st === "stale"; width: parent.width; opacity: ip.st === "stale" ? 0.62 : 1
            Traj { name: "tray-0412"; time: "14:31:07"; ring: Demo.focusRing }
            Traj { name: "tray-0411"; time: "14:30:52" }
            Traj { name: "tray-0410"; time: "14:29:40" } }
        NoteBox { visible: ip.st === "stale"; icon: "recents"; text: "Última lectura hace 18 min." }
        Column { visible: ip.st === "offline"; width: parent.width
            Traj { unread: true; name: "Trayectorias de Cortex"; detail: "sin lectura mientras archied no responda"; time: "hace 2 min" } }
        Column { visible: ip.st === "degraded"; width: parent.width
            Traj { name: "tray-0412"; time: "14:31:07" }
            Traj { unread: true; name: "Trayectorias anteriores"; detail: "bridge sin datos"; time: "" } }
        Item { width: parent.width; height: 24
            Eyebrow { text: "Estado interno de IRIS"; anchors.verticalCenter: parent.verticalCenter; y: 6 }
            Chip { tone: "unk"; icon: "unknown"; text: "sin lectura"; anchors.right: parent.right; anchors.verticalCenter: parent.verticalCenter } }
        Item { width: 1; height: 10 }
        VoidBox { icon: "lock"; title: "No hay forma de leerlo"; text: "IRIS no expone una API de lectura, así que no se dibuja ningún estado, pensamiento ni telemetría." }
    }
    FootHints { id: foot; y: col.y + col.implicitHeight + 16; hints: [["↑↓", "elegir"], ["Esc", "cerrar"]] }
}
