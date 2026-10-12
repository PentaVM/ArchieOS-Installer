import QtQuick

// Forge, 1000 px window. DESIGN SAMPLE. The catalogue of bridges is shown with the states a real
// one would have; the pipeline stages are drawn apart and marked "Diseño · sin worker": the
// Forge module has no worker, so no stage shows progress.
AppWindow {
    id: fg
    title: "Forge"
    property string st: Demo.state
    property int tool: 0

    readonly property var bridges: st === "degraded"
        ? [ { n: "freecad-bridge", s: "live", d: "hace 4 s", t: "FreeCAD" }, { n: "kicad-bridge", s: "nodata", d: "—", t: "KiCad" },
            { n: "video-bridge", s: "dead", d: "último estado hace 2 min", t: "Vídeo" }, { n: "cad-export", s: "nodata", d: "—", t: "FreeCAD" } ]
        : [ { n: "freecad-bridge", s: "live", d: "hace 4 s", t: "FreeCAD" }, { n: "kicad-bridge", s: "live", d: "hace 7 min", t: "KiCad" },
            { n: "video-bridge", s: "dead", d: "último estado hace 2 min", t: "Vídeo" }, { n: "cad-export", s: "nodata", d: "—", t: "FreeCAD" } ]
    readonly property var chipOf: ({
        live:   { t: "en vivo",          i: "ok",      k: "ok" },
        dead:   { t: "sin respuesta",    i: "bad",     k: "bad" },
        nodata: { t: "bridge sin datos", i: "warn",    k: "warn" },
        none:   { t: "sin lectura",      i: "unknown", k: "unk" } })

    trailing: [ Chip { design: true; icon: "sample"; text: "Diseño · datos de ejemplo" } ]
    sidebar: [
        Eyebrow { text: "Herramientas"; leftPadding: 10; bottomPadding: 6 },
        SideRow { icon: "cad"; text: "FreeCAD"; on: fg.tool === 0; ring: Demo.focusRing; onClicked: fg.tool = 0 },
        SideRow { icon: "data"; text: "KiCad"; on: fg.tool === 1; onClicked: fg.tool = 1 },
        SideRow { icon: "video"; text: "Vídeo"; on: fg.tool === 2; onClicked: fg.tool = 2 } ]

    Item { width: parent.width; height: 24
        Eyebrow { text: "Catálogo de puentes"; anchors.verticalCenter: parent.verticalCenter }
        StateChip { st: fg.st; noun: "puentes"; anchors.right: parent.right; anchors.verticalCenter: parent.verticalCenter } }

    ErrBox { visible: fg.st === "error"; text: "No se pudo leer el catálogo de puentes: el bridge respondió con un formato no válido." }
    VoidBox { visible: fg.st === "vacio"; icon: "plug"; title: "Ningún puente registrado"; text: "Cuando un bridge se anuncie aparecerá aquí con su estado real." }

    Column { visible: fg.st !== "error" && fg.st !== "vacio"; width: parent.width; spacing: 0; opacity: fg.st === "stale" ? 0.62 : 1
        readonly property real fr: (width - 24 - 140 - 90 - 36) / 2.6
        Item { width: parent.width; height: 24
            Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.hair }
            Repeater { model: [["Puente", 12], ["Estado", 12 + parent.parent.fr * 1.4 + 12], ["Último dato", 12 + parent.parent.fr * 1.4 + 12 + 140 + 12], ["Herramienta", 12 + parent.parent.fr * 2.6 + 12 + 140 + 24]]
                delegate: Text { x: modelData[1]; y: 0; text: modelData[0]; color: Theme.t3; font.family: Theme.font; font.pixelSize: 11; font.weight: Font.Medium } } }
        Item { width: 1; height: 4 }
        Repeater { model: fg.bridges
            delegate: Rectangle { width: parent.width; height: 44; radius: 8; color: (Demo.focusRing && index === 0) ? Theme.sel : "transparent"
                readonly property real fr: (width - 24 - 140 - 90 - 36) / 2.6
                readonly property var spec: fg.chipOf[fg.st === "offline" ? "none" : modelData.s]
                Rectangle { anchors.fill: parent; radius: 8; color: "transparent"; border.width: 2; border.color: Theme.acc; visible: Demo.focusRing && index === 0 }
                Text { x: 12; width: fr * 1.4; anchors.verticalCenter: parent.verticalCenter; elide: Text.ElideRight; text: modelData.n; color: Theme.t1; font.family: "monospace"; font.pixelSize: 12 }
                Chip { x: 12 + fr * 1.4 + 12; anchors.verticalCenter: parent.verticalCenter; small: true; text: spec.t; icon: spec.i; tone: spec.k }
                Text { x: 12 + fr * 1.4 + 12 + 140 + 12; width: fr * 1.2; anchors.verticalCenter: parent.verticalCenter; elide: Text.ElideRight
                    text: fg.st === "offline" ? "—" : modelData.d; color: Theme.t2; font.family: Theme.font; font.pixelSize: 12 }
                Text { x: parent.width - 90 - 12; anchors.verticalCenter: parent.verticalCenter; text: modelData.t; color: Theme.t3; font.family: Theme.font; font.pixelSize: 12 } } } }

    NoteBox { visible: fg.st === "offline"; icon: "stale"; text: "archied no responde: los puentes se muestran sin lectura, no como inactivos." }
    NoteBox { visible: fg.st === "stale"; icon: "recents"; text: "Catálogo leído hace 18 min; los estados pueden haber cambiado." }

    Item { width: parent.width; height: 24
        Eyebrow { text: "Etapas del pipeline"; anchors.verticalCenter: parent.verticalCenter }
        Chip { design: true; icon: "sample"; text: "Diseño · sin worker"; anchors.right: parent.right; anchors.verticalCenter: parent.verticalCenter } }
    Row { width: parent.width; spacing: 0
        Repeater { model: ["Importar", "Validar", "Generar", "Entregar"]
            delegate: Row { spacing: 0
                Item { width: (fg.width - 240 - 3 * 22) / 4; height: 54
                    DashedRect { anchors.fill: parent; radius: 10; color: Qt.rgba(141 / 255, 140 / 255, 156 / 255, 0.55) }
                    Column { x: 12; anchors.verticalCenter: parent.verticalCenter; spacing: 3
                        Text { text: modelData; color: Theme.t2; font.family: Theme.font; font.pixelSize: 12; font.weight: Font.Medium }
                        Text { text: "sin worker"; color: Theme.t3; font.family: Theme.font; font.pixelSize: 11 } } }
                Item { visible: index < 3; width: 22; height: 54; Icon { anchors.centerIn: parent; name: "chevR"; size: 14; color: Theme.t3 } } } } }
    NoteBox { text: "El módulo Forge todavía no tiene worker: las etapas son solo el marco visual y no muestran progreso." }
}
