import QtQuick

// Notification centre, 540 px. DESIGN SAMPLE: nothing feeds it yet. Its three sources are real
// in principle (long jobs from the bridge, Policy audit, Sentry changes), but no adapter joins
// them into one history, so every entry here is an example and says so. Grouping is by origin.
Panel {
    id: nc
    kind: "panel"
    width: 540
    height: foot.y + foot.height
    property string st: Demo.state
    // Keys of the entries each state shows; dismissed ones (Demo.notifGone) are left out.
    readonly property var keys: (st === "normal" || st === "stale") ? ["n1", "n2", "n3", "n4", "n5"]
        : st === "degraded" ? ["n2", "n4", "n6"] : st === "offline" ? ["n1o"] : []
    readonly property int live: { var g = Demo.notifGone, n = 0; for (var i = 0; i < keys.length; i++) if (g.indexOf(keys[i]) < 0) n++; return n }
    readonly property bool allGone: keys.length > 0 && live === 0

    component Item2: Item {
        id: it
        property string glyph
        property color glyphColor: Theme.t2
        property string title
        property string detail
        property real progress: -1
        property string chipText
        property string chipIcon
        property string chipTone: "neutral"
        property string time
        property bool ring: false
        property string nid
        visible: Demo.notifGone.indexOf(nid) < 0
        width: parent ? parent.width : 400
        height: visible ? body.implicitHeight + 20 : 0
        Rectangle { anchors.fill: parent; radius: 10; color: it.ring ? Theme.sel : "transparent"
            border.width: it.ring ? 2 : 0; border.color: Theme.acc }
        Rectangle { id: gl; x: 10; y: 10; width: 32; height: 32; radius: 8; color: Qt.rgba(1, 1, 1, 0.05); border.width: 1; border.color: Theme.hair
            Icon { anchors.centerIn: parent; name: it.glyph; size: 16; color: it.glyphColor } }
        Column { id: body; x: 54; y: 10; width: parent.width - 54 - 78; spacing: 2
            Text { width: parent.width; elide: Text.ElideRight; text: it.title; color: Theme.t1; font.family: Theme.font; font.pixelSize: 13; font.weight: Font.Medium }
            Text { width: parent.width; elide: Text.ElideRight; text: it.detail; color: Theme.t2; font.family: Theme.font; font.pixelSize: 12 }
            Item { visible: it.progress >= 0; width: parent.width; height: 12
                Rectangle { y: 4; width: parent.width; height: 4; radius: 2; color: Qt.rgba(1, 1, 1, 0.10)
                    Rectangle { width: parent.width * it.progress; height: 4; radius: 2; color: Theme.acc } } }
            Item { visible: it.chipText !== ""; width: 1; height: 4 }
            Chip { visible: it.chipText !== ""; small: true; text: it.chipText; icon: it.chipIcon; tone: it.chipTone } }
        Text { anchors.right: parent.right; anchors.rightMargin: 10; y: 10; text: it.time; color: Theme.t3; font.family: Theme.font; font.pixelSize: 11 }
        HoverHandler { id: hov }
        Rectangle { visible: hov.hovered && it.nid !== ""; anchors.right: parent.right; anchors.rightMargin: 8; anchors.bottom: parent.bottom; anchors.bottomMargin: 8
            width: 22; height: 22; radius: 11; color: xma.containsMouse ? Theme.pressed : Theme.hover; border.width: 1; border.color: Theme.hair
            Text { anchors.centerIn: parent; text: "×"; color: Theme.t2; font.family: Theme.font; font.pixelSize: 14 }
            MouseArea { id: xma; anchors.fill: parent; hoverEnabled: true; onClicked: Demo.dismiss(it.nid) } }
    }

    Column { id: col; x: 20; y: 20; width: parent.width - 40; spacing: 0
        Text { text: "Notificaciones"; color: Theme.t1; font.family: Theme.disp; font.pixelSize: 22; font.weight: Font.DemiBold; font.letterSpacing: -0.33 }
        Flow { width: parent.width; topPadding: 12; bottomPadding: 4; spacing: 8
            Chip { design: true; icon: "sample"; text: "Diseño · datos de ejemplo" }
            StateChip { st: nc.st; noun: "avisos" } }
        Row { visible: nc.keys.length > 0; spacing: 8; topPadding: 6; bottomPadding: 2
            Btn { text: "Borrar todo"; dis: nc.live === 0; onClicked: Demo.dismissAll(nc.keys) }
            Btn { text: "Restaurar ejemplos"; dis: Demo.notifGone.length === 0; onClicked: Demo.restoreNotifs() } }
        Item { width: 1; height: 8 }
        ErrBox { visible: nc.st === "error"; text: "No se pudo leer el historial de notificaciones: respuesta no válida del bridge." }
        VoidBox { visible: nc.st === "vacio"; icon: "bell"; title: "Sin notificaciones"; text: "No hay trabajos, decisiones de Policy ni cambios de Sentry que mostrar." }
        VoidBox { visible: nc.allGone; icon: "bell"; title: "Todo al día"; text: "Descartaste todos los avisos de ejemplo. «Restaurar ejemplos» los trae de vuelta." }

        // normal and stale
        Column { visible: (nc.st === "normal" || nc.st === "stale") && !nc.allGone; width: parent.width; opacity: nc.st === "stale" ? 0.62 : 1
            Eyebrow { text: "Hoy"; leftPadding: 10; bottomPadding: 6 }
            Item2 { nid: "n1"; glyph: "activity"; glyphColor: Theme.info; title: "freecad · sweep"; detail: "31 de 50 · motor-bracket · job-7f3a"; progress: 0.62
                chipText: "en curso"; chipIcon: "activity"; chipTone: "info"; time: "ahora"; ring: Demo.focusRing }
            Item2 { nid: "n2"; glyph: "bad"; glyphColor: Theme.bad; title: "Policy rechazó una escritura"; detail: "Fuera del espacio de trabajo"
                chipText: "rechazada"; chipIcon: "bad"; chipTone: "bad"; time: "14:08" }
            Item2 { nid: "n3"; glyph: "done"; glyphColor: Theme.ok; title: "Policy concedió una lectura"; detail: "/archie/workspaces/motor-bracket (1 operación)"
                chipText: "concedida"; chipIcon: "done"; chipTone: "ok"; time: "14:09" }
            Item2 { nid: "n4"; glyph: "dkSentry"; title: "Dispositivo nuevo en la red"; detail: "192.168.1.88 · visto por ssdp"
                chipText: "Sentry"; chipIcon: "dkSentry"; chipTone: "neutral"; time: "14:05" }
            Item2 { nid: "n5"; glyph: "stack"; title: "3 notificaciones más de Sentry"; detail: "Agrupadas: dispositivos vistos de nuevo"; time: "14:02" } }
        Item { visible: nc.st === "stale"; width: 1; height: 12 }
        NoteBox { visible: nc.st === "stale"; icon: "recents"; text: "Última lectura hace 18 min; pueden faltar avisos recientes." }

        // offline: nothing is updated; the job's progress is unknown, not zero
        Column { visible: nc.st === "offline"; width: parent.width
            Item2 { nid: "n1o"; glyph: "unknown"; title: "freecad · sweep"; detail: "sin lectura del avance"; chipText: "no medido"; chipIcon: "unknown"; chipTone: "unk"; time: "hace 2 min" }
            Item { width: 1; height: 8 }
            NoteBox { text: "Los trabajos y las decisiones de Policy no se actualizan mientras archied no responda." } }

        // degraded: what arrived is shown; what did not is marked, not hidden
        Column { visible: nc.st === "degraded"; width: parent.width
            Item2 { nid: "n2"; glyph: "bad"; glyphColor: Theme.bad; title: "Policy rechazó una escritura"; detail: "Fuera del espacio de trabajo"
                chipText: "rechazada"; chipIcon: "bad"; chipTone: "bad"; time: "14:08" }
            Item2 { nid: "n4"; glyph: "dkSentry"; title: "Dispositivo nuevo en la red"; detail: "192.168.1.88 · visto por ssdp"
                chipText: "Sentry"; chipIcon: "dkSentry"; chipTone: "neutral"; time: "14:05" }
            Item2 { nid: "n6"; glyph: "unknown"; title: "Trabajos largos"; detail: "bridge sin datos"; chipText: "no medido"; chipIcon: "unknown"; chipTone: "unk" } }
    }
    FootHints { id: foot; y: col.y + col.implicitHeight + 16; hints: [["↑↓", "elegir"], ["Esc", "cerrar"]] }
}
